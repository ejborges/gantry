//! Single sign-on with OpenID Connect (authorization code flow with PKCE, state and nonce).
//! Works with any standards-compliant provider: Microsoft Entra ID, Okta, Keycloak, Ping,
//! Google Workspace, ADFS 2016+, Authentik, Dex. SAML identity providers are handled in
//! `saml.rs`; both share the account matching and provisioning rules in [`resolve_user`].
//!
//! ID tokens are verified against the provider's published keys: signature, issuer, audience,
//! expiry and nonce. Only asymmetric algorithms are accepted.

use crate::auth;
use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, ReqInfo};
use crate::{audit, crypto, AppState, St};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

const ALLOWED_ALGS: &[Algorithm] = &[Algorithm::RS256, Algorithm::RS384, Algorithm::RS512, Algorithm::PS256, Algorithm::PS384, Algorithm::PS512, Algorithm::ES256, Algorithm::ES384];

/// The part of a sign-in provider's settings that decides which account a verified identity maps to.
pub struct SsoProvider {
    pub id: Uuid,
    pub name: String,
    pub auto_provision: bool,
    pub link_by_email: bool,
    pub allowed_domains: Vec<String>,
    pub default_role: String,
}

/// An identity an external provider has vouched for.
pub struct SsoIdentity {
    /// Stable identifier at the provider (OIDC `sub`, SAML `NameID`).
    pub subject: String,
    pub email: Option<String>,
    /// The provider explicitly said the address is not verified.
    pub email_unverified: bool,
    pub name: Option<String>,
}

struct Provider {
    sso: SsoProvider,
    issuer: String,
    client_id: String,
    client_secret: String,
    scopes: String,
}

async fn provider(st: &AppState, id: Uuid) -> AppResult<Provider> {
    let r = sqlx::query("SELECT id, name, issuer, client_id, client_secret_enc, scopes, auto_provision, link_by_email, allowed_domains, default_role FROM auth_providers WHERE id = $1 AND enabled AND kind = 'oidc'").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let client_secret = st.secrets.open(&r.get::<String, _>("client_secret_enc")).ok_or_else(|| AppError::Other(anyhow::anyhow!("the client secret for this sign-in provider cannot be decrypted")))?;
    Ok(Provider {
        sso: SsoProvider { id: r.get("id"), name: r.get("name"), auto_provision: r.get("auto_provision"), link_by_email: r.get("link_by_email"), allowed_domains: r.get("allowed_domains"), default_role: r.get("default_role") },
        issuer: r.get("issuer"),
        client_id: r.get("client_id"),
        client_secret,
        scopes: r.get("scopes"),
    })
}

/// GET a JSON document from an identity provider, through the outbound address checks.
async fn fetch_json(st: &AppState, url: &str) -> anyhow::Result<Value> {
    let (client, url) = crate::integrations::guarded_client(st, url).await?;
    Ok(client.get(url).send().await.map_err(|e| anyhow::anyhow!("{}", e.without_url()))?.error_for_status().map_err(|e| anyhow::anyhow!("{}", e.without_url()))?.json().await?)
}

fn check_scheme(st: &AppState, u: &str) -> anyhow::Result<()> {
    anyhow::ensure!(u.starts_with("https://") || (st.cfg.security.allow_insecure_outbound && u.starts_with("http://")), "identity provider URLs must use https");
    Ok(())
}

async fn discovery(st: &AppState, issuer: &str) -> anyhow::Result<Value> {
    check_scheme(st, issuer)?;
    // Identity provider addresses come from an administrator, and the discovery document
    // names more of them. Each goes through the same checks as any outbound request, so
    // sign-in settings cannot be used to reach the loopback interface or a metadata service.
    let doc: Value = fetch_json(st, &format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'))).await?;
    anyhow::ensure!(doc["issuer"].as_str().map(|i| i.trim_end_matches('/')) == Some(issuer.trim_end_matches('/')), "the discovery document names a different issuer");
    for k in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
        check_scheme(st, doc[k].as_str().ok_or_else(|| anyhow::anyhow!("the discovery document has no {k}"))?)?;
    }
    Ok(doc)
}

fn base_url(st: &AppState, headers: &HeaderMap) -> String {
    if !st.cfg.server.public_url.is_empty() {
        return st.cfg.server.public_url.trim_end_matches('/').to_string();
    }
    format!("http://{}", headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost"))
}

fn redirect_uri(st: &AppState, headers: &HeaderMap, id: Uuid) -> String {
    format!("{}/api/v1/auth/oidc/{id}/callback", base_url(st, headers))
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(crypto::sha256(verifier.as_bytes()))
}

fn redirect(to: &str) -> Response {
    let mut r = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(to) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r
}

/// Sign-in options for the login page. Public.
async fn providers(State(st): State<St>) -> AppResult<Json<Value>> {
    let list: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'name', name, 'kind', kind, 'start', '/api/v1/auth/' || kind || '/' || id || '/start') ORDER BY name), '[]'::jsonb) FROM auth_providers WHERE enabled").fetch_one(&st.db).await?;
    let site = crate::settings::get(&st, "site").await;
    let legal = crate::settings::get(&st, "legal").await;
    let documents: Vec<&str> = crate::settings::LEGAL_DOCS.iter().map(|d| d.0).filter(|k| legal[*k].as_str().is_some_and(|t| !t.trim().is_empty())).collect();
    Ok(Json(json!({"local_login": st.cfg.auth.allow_local_login, "providers": list,
        "site": {"name": site["name"], "login_notice": site["login_notice"], "support": site["support"]}, "legal": documents})))
}

async fn start(State(st): State<St>, headers: HeaderMap, Path(id): Path<Uuid>) -> AppResult<Response> {
    let p = provider(&st, id).await?;
    let doc = discovery(&st, &p.issuer).await.map_err(|e| AppError::Upstream(e.to_string()))?;
    let (state, nonce, verifier) = (crypto::random_token(24), crypto::random_token(24), crypto::random_token(48));
    sqlx::query("INSERT INTO oidc_states (state, provider_id, nonce, pkce_verifier) VALUES ($1,$2,$3,$4)").bind(&state).bind(p.sso.id).bind(&nonce).bind(&verifier).execute(&st.db).await?;
    let mut url = url::Url::parse(doc["authorization_endpoint"].as_str().unwrap_or_default()).map_err(|e| AppError::Upstream(e.to_string()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &p.client_id)
        .append_pair("redirect_uri", &redirect_uri(&st, &headers, id))
        .append_pair("scope", &p.scopes)
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &pkce_challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    Ok(redirect(url.as_str()))
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<Value>,
    name: Option<String>,
    preferred_username: Option<String>,
}

/// Verify an ID token against a JWKS. Exposed for tests.
pub fn verify_id_token(token: &str, jwks: &JwkSet, issuer: &str, client_id: &str) -> anyhow::Result<Value> {
    let head = decode_header(token)?;
    anyhow::ensure!(ALLOWED_ALGS.contains(&head.alg), "ID token uses a disallowed algorithm");
    let jwk = match &head.kid {
        Some(kid) => jwks.find(kid).ok_or_else(|| anyhow::anyhow!("ID token was signed with an unknown key"))?,
        None => {
            anyhow::ensure!(jwks.keys.len() == 1, "ID token names no key and the provider publishes several");
            &jwks.keys[0]
        }
    };
    let mut v = Validation::new(head.alg);
    v.set_audience(&[client_id]);
    v.set_issuer(&[issuer, issuer.trim_end_matches('/')]);
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    v.leeway = 60;
    Ok(decode::<Value>(token, &DecodingKey::from_jwk(jwk)?, &v)?.claims)
}

async fn exchange(st: &AppState, p: &Provider, headers: &HeaderMap, code: &str, verifier: &str, nonce: &str) -> anyhow::Result<Claims> {
    let doc = discovery(st, &p.issuer).await?;
    // The client secret goes only to an address that passed the outbound checks.
    let (client, token_url) = crate::integrations::guarded_client(st, doc["token_endpoint"].as_str().unwrap_or_default()).await?;
    let token: Value = client
        .post(token_url)
        .form(&[("grant_type", "authorization_code"), ("code", code), ("redirect_uri", &redirect_uri(st, headers, p.sso.id)), ("client_id", &p.client_id), ("client_secret", &p.client_secret), ("code_verifier", verifier)])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!("token endpoint refused the code: {}", e.without_url()))?
        .json()
        .await?;
    let id_token = token["id_token"].as_str().ok_or_else(|| anyhow::anyhow!("the provider returned no ID token"))?;
    let jwks: JwkSet = serde_json::from_value(fetch_json(st, doc["jwks_uri"].as_str().unwrap_or_default()).await?)?;
    let claims: Claims = serde_json::from_value(verify_id_token(id_token, &jwks, &p.issuer, &p.client_id)?)?;
    anyhow::ensure!(claims.nonce.as_deref().map(|n| crypto::ct_eq(n.as_bytes(), nonce.as_bytes())).unwrap_or(false), "ID token nonce does not match this sign-in attempt");
    Ok(claims)
}

/// Find or create the account for an identity a provider has vouched for. The same rules
/// apply to OpenID Connect and SAML: an identity seen before maps to the same account; a new
/// one is linked to an existing account by email only if the provider allows it; a new account
/// is created only if the provider allows it and the email domain is permitted.
pub async fn resolve_user(st: &AppState, p: &SsoProvider, c: &SsoIdentity) -> AppResult<Uuid> {
    if let Some(uid) = sqlx::query_scalar::<_, Uuid>("SELECT i.user_id FROM identities i JOIN users u ON u.id = i.user_id WHERE i.provider_id = $1 AND i.subject = $2 AND u.active").bind(p.id).bind(&c.subject).fetch_optional(&st.db).await? {
        return Ok(uid);
    }
    if sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM identities WHERE provider_id = $1 AND subject = $2)").bind(p.id).bind(&c.subject).fetch_one(&st.db).await? {
        return Err(AppError::Forbidden("this account has been deactivated".into()));
    }
    let email = c.email.as_deref().map(|e| e.trim().to_lowercase()).filter(|e| crate::accounts::valid_email(e)).ok_or_else(|| AppError::Forbidden("the identity provider did not supply an email address".into()))?;
    // An explicit "not verified" is never trusted for linking or provisioning.
    if c.email_unverified {
        return Err(AppError::Forbidden("the identity provider reports this email address as unverified".into()));
    }
    let domain = email.rsplit('@').next().unwrap_or_default();
    if !p.allowed_domains.is_empty() && !p.allowed_domains.iter().any(|d| d.eq_ignore_ascii_case(domain)) {
        return Err(AppError::Forbidden("this email domain is not allowed to sign in".into()));
    }
    let existing = sqlx::query("SELECT id, active, kind FROM users WHERE lower(email) = $1").bind(&email).fetch_optional(&st.db).await?;
    let uid = match existing {
        // People sign in; AI agent accounts only ever authenticate with tokens.
        Some(r) if r.get::<String, _>("kind") != "human" => return Err(AppError::Forbidden("this address belongs to an AI agent account, which cannot sign in".into())),
        Some(r) if !r.get::<bool, _>("active") => return Err(AppError::Forbidden("this account has been deactivated".into())),
        Some(_) if !p.link_by_email => return Err(AppError::Forbidden("an account with this email exists but is not linked to this sign-in provider".into())),
        Some(r) => r.get("id"),
        None if !p.auto_provision => return Err(AppError::Forbidden("no account exists for you yet; ask an administrator".into())),
        None => {
            let name = c.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| email.clone());
            let uid: Uuid = sqlx::query_scalar("INSERT INTO users (email, name) VALUES ($1, $2) RETURNING id").bind(&email).bind(name.trim().chars().filter(|c| !crate::accounts::deceptive(*c)).take(200).collect::<String>()).fetch_one(&st.db).await?;
            sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type) SELECT id, $1, 'global' FROM roles WHERE key = $2 AND scope = 'global'").bind(uid).bind(&p.default_role).execute(&st.db).await?;
            audit::log_system(st, None, &format!("sso:{}", p.name), "user.provision", "user", &uid.to_string(), "success", json!({"email": email, "role": p.default_role})).await;
            uid
        }
    };
    sqlx::query("INSERT INTO identities (provider_id, subject, user_id) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING").bind(p.id).bind(&c.subject).bind(uid).execute(&st.db).await?;
    Ok(uid)
}

async fn callback(State(st): State<St>, info: ReqInfo, headers: HeaderMap, Path(id): Path<Uuid>, Query(q): Query<Callback>) -> Response {
    let fail = |reason: String| {
        let (st, info) = (st.clone(), info.clone());
        async move {
            audit::security(&st, "auth.sso_failed", "warning", None, &info, json!({"provider": id, "reason": reason})).await;
            redirect("/login?error=sso")
        }
    };
    let p = match provider(&st, id).await {
        Ok(p) => p,
        Err(_) => return fail("unknown or disabled provider".into()).await,
    };
    if let Some(e) = q.error {
        return fail(format!("provider returned error {}", e.chars().take(80).collect::<String>())).await;
    }
    let (Some(code), Some(state)) = (q.code, q.state) else { return fail("missing code or state".into()).await };
    let pending = sqlx::query("DELETE FROM oidc_states WHERE state = $1 AND provider_id = $2 AND created_at > now() - interval '10 minutes' RETURNING nonce, pkce_verifier").bind(&state).bind(id).fetch_optional(&st.db).await;
    let Ok(Some(row)) = pending else { return fail("unknown, reused or expired state".into()).await };
    let claims = match exchange(&st, &p, &headers, &code, &row.get::<String, _>("pkce_verifier"), &row.get::<String, _>("nonce")).await {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()).await,
    };
    let unverified = matches!(&claims.email_verified, Some(Value::Bool(false))) || matches!(&claims.email_verified, Some(Value::String(s)) if s == "false");
    let who = SsoIdentity { subject: claims.sub.clone(), email: claims.email.clone(), email_unverified: unverified, name: claims.name.clone().or(claims.preferred_username.clone()) };
    let uid = match resolve_user(&st, &p.sso, &who).await {
        Ok(u) => u,
        Err(e) => return fail(e.public_message()).await,
    };
    match auth::start_session(&st, uid, &format!("oidc:{}", p.sso.name), &info).await {
        Ok(cookie) => {
            audit::log_system(&st, Some(&info), claims.email.as_deref().unwrap_or(&claims.sub), "auth.login", "user", &uid.to_string(), "success", json!({"method": "oidc", "provider": p.sso.name})).await;
            let mut r = redirect("/");
            r.headers_mut().insert(header::SET_COOKIE, cookie);
            r
        }
        Err(e) => fail(e.public_message()).await,
    }
}

/// Validate provider settings before they are stored (used by the administration API).
pub async fn check_provider(st: &AppState, issuer: &str) -> AppResult<()> {
    discovery(st, issuer).await.map(|_| ()).map_err(|e| bad(format!("could not read the provider's discovery document: {e}")))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/auth/providers", "Auth", "Sign-in options for the login page", get(providers))
        .public()
        .route("GET", "/api/v1/auth/oidc/{id}/start", "Auth", "Begin single sign-on (redirects to the identity provider)", get(start))
        .public()
        .route("GET", "/api/v1/auth/oidc/{id}/callback", "Auth", "Complete single sign-on", get(callback))
        .public()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_s256_known_answer() {
        // RFC 7636 appendix B.
        assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn symmetric_and_unsigned_tokens_are_refused() {
        let jwks: JwkSet = serde_json::from_value(json!({"keys": []})).unwrap();
        // {"alg":"HS256","typ":"JWT"} . {"sub":"x"} . signature
        let hs = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ4In0.c2ln";
        assert!(verify_id_token(hs, &jwks, "https://idp", "client").unwrap_err().to_string().contains("disallowed"));
        // {"alg":"none"} is not even a parsable header for the JWT library.
        let none = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ4In0.";
        assert!(verify_id_token(none, &jwks, "https://idp", "client").is_err());
    }
}
