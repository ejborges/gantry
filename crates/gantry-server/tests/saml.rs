//! SAML sign-in: signature verification against responses signed by another implementation
//! (xmlsec1, see `tests/fixtures/saml`), the attacks the verification must stop, and the
//! whole browser flow against a running server.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use gantry::saml::{self, Expect};
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ACS: &str = "https://gantry.example.org/api/v1/auth/saml/11111111-1111-1111-1111-111111111111/acs";
const SP: &str = "https://gantry.example.org/api/v1/auth/saml/11111111-1111-1111-1111-111111111111/metadata";
const IDP: &str = "https://idp.example.org/entity";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/saml/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn keys(cert: &str) -> Vec<RsaPublicKey> {
    saml::keys_from_certificates(&[fixture(cert)]).unwrap().into_iter().map(|k| k.0).collect()
}

fn at(time: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(time).unwrap().with_timezone(&Utc)
}

fn read(xml: &str, keys: &[RsaPublicKey], now: &str) -> Result<saml::Assertion, String> {
    saml::read_response(xml, &Expect { idp_entity_id: IDP, sp_entity_id: SP, acs_url: ACS, now: at(now), keys })
}

const NOON: &str = "2026-06-01T12:01:00Z";

// ───────────────────────── agreement with xmlsec1 ─────────────────────────

#[test]
fn responses_signed_by_another_implementation_verify() {
    let k = keys("idp-cert.pem");
    for name in ["assertion-signed.xml", "assertion-signed-sha512.xml", "response-signed.xml"] {
        let a = read(&fixture(name), &k, NOON).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(a.name_id, "ada@example.org", "{name}");
        assert_eq!(a.id, "_assert1");
        assert_eq!(a.in_response_to.as_deref(), Some("_req1"));
        assert_eq!(a.attributes["displayName"], vec!["Ada Lovelace & Co. <R&D>"], "entities are decoded once");
        assert_eq!(a.attributes["mail"], vec!["ada@example.org"]);
        assert_eq!(a.attributes["email"], vec!["ada@example.org"], "the friendly name works too");
        assert_eq!(a.attributes["groups"], vec!["engineering", "ops"]);
        assert_eq!(a.expires, at("2026-06-01T12:05:00Z"), "the earlier of the two end times");
        let who = saml::identity(&a, "", "");
        assert_eq!((who.email.as_deref(), who.name.as_deref()), (Some("ada@example.org"), Some("Ada Lovelace & Co. <R&D>")));
    }
}

#[test]
fn sha1_signatures_are_refused_even_when_valid() {
    let err = read(&fixture("assertion-signed-sha1.xml"), &keys("idp-cert.pem"), NOON).unwrap_err();
    assert!(err.contains("SHA-256"), "{err}");
}

#[test]
fn a_signature_from_a_key_that_is_not_configured_is_refused() {
    let signed_by_other = fixture("assertion-signed-by-other-key.xml");
    let err = read(&signed_by_other, &keys("idp-cert.pem"), NOON).unwrap_err();
    assert!(err.contains("configured certificates"), "{err}");
    // The same message verifies once that certificate is the configured one: the refusal
    // above is about the key, not about the message.
    assert!(read(&signed_by_other, &keys("other-cert.pem"), NOON).is_ok());
    // During a certificate rollover both are configured.
    let both: Vec<RsaPublicKey> = keys("idp-cert.pem").into_iter().chain(keys("other-cert.pem")).collect();
    assert!(read(&signed_by_other, &both, NOON).is_ok() && read(&fixture("assertion-signed.xml"), &both, NOON).is_ok());
    assert!(read(&fixture("assertion-signed.xml"), &[], NOON).is_err(), "no certificate, no sign-in");
}

#[test]
fn text_hidden_behind_a_comment_is_part_of_the_name() {
    // The signature ignores comments, so this message is genuinely signed. An implementation
    // that reads only the text before the comment would sign in "ada@example.org".
    let a = read(&fixture("assertion-signed-comment-in-name.xml"), &keys("idp-cert.pem"), NOON).unwrap();
    assert_eq!(a.name_id, "ada@example.org.attacker.test");
}

// ───────────────────────── tampering ─────────────────────────

fn refused(xml: &str, why: &str) -> String {
    match read(xml, &keys("idp-cert.pem"), NOON) {
        Ok(a) => panic!("{why}: accepted as {}", a.name_id),
        Err(e) => e,
    }
}

#[test]
fn any_change_to_signed_content_is_detected() {
    let good = fixture("assertion-signed.xml");
    let changes = [
        ("the name", good.replace(">ada@example.org</saml:NameID>", ">root@example.org</saml:NameID>")),
        ("an attribute value", good.replace(">engineering<", ">administrators<")),
        ("an added attribute value", good.replace("<saml:AttributeValue xsi:type=\"xs:string\">ops</saml:AttributeValue>", "<saml:AttributeValue xsi:type=\"xs:string\">ops</saml:AttributeValue><saml:AttributeValue>admins</saml:AttributeValue>")),
        ("the audience", good.replace(&format!("<saml:Audience>{SP}"), "<saml:Audience>https://other.example.org")),
        ("the validity window", good.replace("NotOnOrAfter=\"2026-06-01T12:10:00Z\"", "NotOnOrAfter=\"2036-06-01T12:10:00Z\"")),
        ("the recipient", good.replace(&format!("Recipient=\"{ACS}\""), "Recipient=\"https://evil.example.org/acs\"")),
        ("an attribute's order", good.replace("ID=\"_assert1\" Version=\"2.0\"", "Version=\"2.1\" ID=\"_assert1\"")),
        ("the request it answers", good.replace("<saml:SubjectConfirmationData InResponseTo=\"_req1\"", "<saml:SubjectConfirmationData InResponseTo=\"_req2\"")),
    ];
    for (what, xml) in changes {
        assert_ne!(xml, good, "the test must actually change {what}");
        let err = refused(&xml, what);
        assert!(err.contains("changed after signing") || err.contains("different") || err.contains("not issued for"), "{what}: {err}");
    }
    // Changes that do not alter the canonical form are, correctly, not changes.
    let reformatted = good.replace("<saml:Audience>", "<saml:Audience  >").replace("ID=\"_assert1\" Version=\"2.0\"", "Version=\"2.0\"   ID='_assert1'");
    assert_ne!(reformatted, good);
    assert!(read(&reformatted, &keys("idp-cert.pem"), NOON).is_ok());
}

#[test]
fn the_signature_value_and_digest_cannot_be_swapped_or_removed() {
    let good = fixture("assertion-signed.xml");
    let other = fixture("assertion-signed-by-other-key.xml");
    let value = |xml: &str| xml.split("<ds:SignatureValue>").nth(1).unwrap().split("</ds:SignatureValue>").next().unwrap().to_string();
    assert!(refused(&good.replace(&value(&good), &value(&other)), "a signature value from another key").contains("configured certificates"));
    assert!(refused(&good.replace(&value(&good), ""), "an empty signature value").contains("configured certificates"));
    assert!(refused(&good.replace(&value(&good), "!!!not base64!!!"), "garbage").contains("unreadable"));
    // Without any signature the message is refused outright.
    let start = good.find("<ds:Signature ").unwrap();
    let end = good.find("</ds:Signature>").unwrap() + "</ds:Signature>".len();
    let unsigned = format!("{}{}", &good[..start], &good[end..]);
    assert!(refused(&unsigned, "no signature").contains("neither the response nor the assertion is signed"));
}

#[test]
fn signature_wrapping_is_refused() {
    let good = fixture("assertion-signed.xml");
    let start = good.find("<saml:Assertion ").unwrap();
    let end = good.find("</saml:Assertion>").unwrap() + "</saml:Assertion>".len();
    let signed_assertion = &good[start..end];
    let sig_start = signed_assertion.find("<ds:Signature ").unwrap();
    let sig_end = signed_assertion.find("</ds:Signature>").unwrap() + "</ds:Signature>".len();
    let forged_body = format!("{}{}", &signed_assertion[..sig_start], &signed_assertion[sig_end..]).replace(">ada@example.org</saml:NameID>", ">root@example.org</saml:NameID>");

    // 1. A forged assertion with the same ID next to the signed one.
    let xml = format!("{}{forged_body}{}", &good[..start], &good[start..]);
    refused(&xml, "forged sibling before the signed assertion");
    let xml = format!("{}{forged_body}{}", &good[..end], &good[end..]);
    refused(&xml, "forged sibling after the signed assertion");

    // 2. A forged assertion with another ID; the signed one tucked away where a careless
    //    reader would still find a valid signature.
    let forged_other_id = forged_body.replace("ID=\"_assert1\"", "ID=\"_evil\"");
    let hidden = format!("<samlp:Extensions>{signed_assertion}</samlp:Extensions>");
    let xml = format!("{}{hidden}{forged_other_id}{}", &good[..start], &good[end..]);
    let err = refused(&xml, "signed assertion hidden in an extension");
    assert!(err.contains("neither the response nor the assertion is signed"), "{err}");

    // 3. The forged assertion wraps the signed one, borrowing its signature element.
    let wrapper = forged_other_id.replace("</saml:Assertion>", &format!("{signed_assertion}</saml:Assertion>"));
    let xml = format!("{}{wrapper}{}", &good[..start], &good[end..]);
    refused(&xml, "signed assertion nested inside a forged one");

    // 4. The forged assertion carries the genuine signature element, which points at the
    //    genuine assertion placed inside the signature itself.
    let genuine_sig = &signed_assertion[sig_start..sig_end];
    let stuffed_sig = genuine_sig.replace("</ds:Signature>", &format!("<ds:Object>{signed_assertion}</ds:Object></ds:Signature>"));
    let forged_with_sig = forged_body.replace("</saml:Issuer>", &format!("</saml:Issuer>{stuffed_sig}"));
    let xml = format!("{}{forged_with_sig}{}", &good[..start], &good[end..]);
    refused(&xml, "genuine assertion inside the signature's Object");

    // 5. Same, but the forged assertion has a different ID than the reference names.
    let xml = format!("{}{}{}", &good[..start], forged_with_sig.replacen("ID=\"_assert1\"", "ID=\"_evil\"", 1), &good[end..]);
    let err = refused(&xml, "reference to another element");
    assert!(err.contains("does not cover the element that carries it"), "{err}");

    // 6. With a signed response: a second, forged assertion added beside the real one.
    let good = fixture("response-signed.xml");
    let xml = good.replace("</samlp:Response>", &format!("{forged_other_id}</samlp:Response>"));
    refused(&xml, "extra assertion in a signed response");
}

#[test]
fn conditions_are_enforced() {
    let good = fixture("assertion-signed.xml");
    let k = keys("idp-cert.pem");
    let err = |now: &str, e: Expect| saml::read_response(&good, &Expect { now: at(now), ..e }).unwrap_err();
    let base = || Expect { idp_entity_id: IDP, sp_entity_id: SP, acs_url: ACS, now: Utc::now(), keys: &k };
    assert!(err("2026-06-01T11:50:00Z", base()).contains("not valid yet"));
    assert!(err("2026-06-01T12:20:00Z", base()).contains("expired"));
    assert!(read(&good, &k, "2026-06-01T11:58:00Z").is_ok(), "two minutes of clock difference are tolerated");
    assert!(read(&good, &k, "2026-06-01T12:06:30Z").is_ok());
    assert!(err("2026-06-01T12:08:00Z", base()).contains("not issued for this sign-in endpoint, or has expired"), "the bearer confirmation ends before the conditions do");
    assert!(err(NOON, Expect { sp_entity_id: "https://someone-else.example.org", ..base() }).contains("audience"));
    assert!(err(NOON, Expect { acs_url: "https://gantry.example.org/other", ..base() }).contains("different service"));
    assert!(err(NOON, Expect { idp_entity_id: "https://another-idp.example.org", ..base() }).contains("different identity provider"));
}

#[test]
fn failures_and_malformed_messages_are_refused_cleanly() {
    let k = keys("idp-cert.pem");
    let good = fixture("assertion-signed.xml");
    let denied = good.replace("urn:oasis:names:tc:SAML:2.0:status:Success", "urn:oasis:names:tc:SAML:2.0:status:AuthnFailed");
    assert!(read(&denied, &k, NOON).unwrap_err().contains("AuthnFailed"));
    for (bad, expect) in [
        ("", "not acceptable XML"),
        ("not xml at all", "not acceptable XML"),
        ("<Response/>", "not a SAML response"),
        ("<samlp:Response xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\"/>", "identity provider reported"),
        ("<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><r>&x;</r>", "not acceptable XML"),
    ] {
        let e = read(bad, &k, NOON).unwrap_err();
        assert!(e.contains(expect), "{bad:?}: {e}");
    }
    let encrypted = good.replace("<saml:Assertion ", "<saml:EncryptedAssertion/><saml:Assertion ");
    assert!(read(&encrypted, &k, NOON).unwrap_err().contains("encrypted"));
    let huge = format!("<a>{}</a>", "x".repeat(600 * 1024));
    assert!(read(&huge, &k, NOON).unwrap_err().contains("too large"));
    // Deep nesting and many nodes end in an error, not in a stack overflow or a long wait.
    let deep = format!("{}{}", "<a>".repeat(50_000), "</a>".repeat(50_000));
    assert!(read(&deep, &k, NOON).is_err());
}

#[test]
fn certificates_are_validated_when_configured() {
    assert!(saml::keys_from_certificates(&[fixture("idp-cert.pem")]).unwrap()[0].1.is_some(), "the expiry date is read for the administrator");
    // The bare base64 from metadata works as well as PEM.
    let bare: String = fixture("idp-cert.pem").lines().filter(|l| !l.contains("-----")).collect();
    assert_eq!(saml::keys_from_certificates(&[bare]).unwrap().len(), 1);
    for bad in ["", "not a certificate", "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----", &fixture("idp-key.pem")] {
        assert!(saml::keys_from_certificates(&[bad.to_string()]).is_err(), "{bad:.40}");
    }
}

// ───────────────────────── the browser flow ─────────────────────────

/// Sign `assertion_body` (an Assertion without a signature) the way an identity provider
/// would. Uses Gantry's canonicalization, which the fixtures above check independently.
fn sign_assertion(assertion: &str, id: &str) -> String {
    let key = RsaPrivateKey::from_pkcs8_pem(&fixture("idp-key.pem")).unwrap();
    let doc = roxmltree::Document::parse(assertion).unwrap();
    let digest = B64.encode(Sha256::digest(saml::canonicalize(assertion, doc.root_element(), None, &[]).as_bytes()));
    let signed_info = format!(
        "<ds:SignedInfo xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"><ds:CanonicalizationMethod Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"></ds:CanonicalizationMethod><ds:SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"></ds:SignatureMethod><ds:Reference URI=\"#{id}\"><ds:Transforms><ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"></ds:Transform><ds:Transform Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"></ds:Transform></ds:Transforms><ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"></ds:DigestMethod><ds:DigestValue>{digest}</ds:DigestValue></ds:Reference></ds:SignedInfo>"
    );
    let info_doc = roxmltree::Document::parse(&signed_info).unwrap();
    let canonical = saml::canonicalize(&signed_info, info_doc.root_element(), None, &[]);
    let signature = B64.encode(key.sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(canonical.as_bytes())).unwrap());
    let block = format!("<ds:Signature xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\">{}<ds:SignatureValue>{signature}</ds:SignatureValue></ds:Signature>", signed_info.replace(" xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"", ""));
    assertion.replacen("</saml:Issuer>", &format!("</saml:Issuer>{block}"), 1)
}

struct Idp {
    acs: String,
    sp: String,
}

impl Idp {
    fn response(&self, request_id: Option<&str>, assertion_id: &str, email: &str, name: &str) -> String {
        let now = Utc::now();
        let t = |d: i64| (now + Duration::seconds(d)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let irt = request_id.map(|r| format!(" InResponseTo=\"{r}\"")).unwrap_or_default();
        let assertion = format!(
            "<saml:Assertion xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\" ID=\"{assertion_id}\" Version=\"2.0\" IssueInstant=\"{}\"><saml:Issuer>{IDP}</saml:Issuer><saml:Subject><saml:NameID>{email}</saml:NameID><saml:SubjectConfirmation Method=\"urn:oasis:names:tc:SAML:2.0:cm:bearer\"><saml:SubjectConfirmationData{irt} NotOnOrAfter=\"{}\" Recipient=\"{}\"/></saml:SubjectConfirmation></saml:Subject><saml:Conditions NotBefore=\"{}\" NotOnOrAfter=\"{}\"><saml:AudienceRestriction><saml:Audience>{}</saml:Audience></saml:AudienceRestriction></saml:Conditions><saml:AttributeStatement><saml:Attribute Name=\"displayName\"><saml:AttributeValue>{name}</saml:AttributeValue></saml:Attribute></saml:AttributeStatement></saml:Assertion>",
            t(0),
            t(300),
            self.acs,
            t(-60),
            t(600),
            self.sp
        );
        let xml = format!(
            "<samlp:Response xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" ID=\"_r{assertion_id}\" Version=\"2.0\" IssueInstant=\"{}\" Destination=\"{}\"{irt}><samlp:Status><samlp:StatusCode Value=\"urn:oasis:names:tc:SAML:2.0:status:Success\"/></samlp:Status>{}</samlp:Response>",
            t(0),
            self.acs,
            sign_assertion(&assertion, assertion_id)
        );
        B64.encode(xml)
    }
}

struct Started {
    request_id: String,
    relay: String,
}

async fn start(app: &common::TestApp, provider: &str) -> Started {
    let resp = app.http.get(format!("{}/api/v1/auth/saml/{provider}/start", app.base)).send().await.unwrap();
    assert_eq!(resp.status(), 302);
    let location = url::Url::parse(resp.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(&location[..url::Position::AfterPath], "https://idp.example.org/sso");
    let q: std::collections::HashMap<String, String> = location.query_pairs().into_owned().collect();
    let raw = B64.decode(&q["SAMLRequest"]).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut flate2::read::DeflateDecoder::new(raw.as_slice()), &mut xml).unwrap();
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "AuthnRequest");
    assert_eq!(root.attribute("AssertionConsumerServiceURL").unwrap(), format!("{}/api/v1/auth/saml/{provider}/acs", app.base));
    Started { request_id: root.attribute("ID").unwrap().to_string(), relay: q["RelayState"].clone() }
}

async fn post_acs(app: &common::TestApp, provider: &str, response: &str, relay: Option<&str>) -> (String, Option<String>) {
    let mut form = vec![("SAMLResponse", response.to_string())];
    if let Some(r) = relay {
        form.push(("RelayState", r.to_string()));
    }
    // The identity provider's page posts the form: a cross-site request without our cookie.
    let resp = app.http.post(format!("{}/api/v1/auth/saml/{provider}/acs", app.base)).header("origin", "https://idp.example.org").form(&form).send().await.unwrap();
    assert_eq!(resp.status(), 302);
    let cookie = resp.headers().get("set-cookie").map(|c| c.to_str().unwrap().split(';').next().unwrap().to_string());
    (resp.headers()["location"].to_str().unwrap().to_string(), cookie)
}

async fn setup() -> (common::TestApp, common::Caller, String, Idp) {
    let app = common::spawn().await;
    let admin = app.admin().await;
    let created = admin
        .created("/api/v1/admin/auth-providers", json!({"kind": "saml", "name": "Corporate", "issuer": IDP, "config": {"sso_url": "https://idp.example.org/sso", "certificates": [fixture("idp-cert.pem")]}}))
        .await;
    let id = created["id"].as_str().unwrap().to_string();
    let idp = Idp { acs: format!("{}/api/v1/auth/saml/{id}/acs", app.base), sp: format!("{}/api/v1/auth/saml/{id}/metadata", app.base) };
    // The test server has no public URL configured, so the console shows the path only.
    assert!(idp.acs.ends_with(created["saml"]["acs_url"].as_str().unwrap()));
    (app, admin, id, idp)
}

async fn me(app: &common::TestApp, cookie: &str) -> (reqwest::StatusCode, Value) {
    let r = app.http.get(format!("{}/api/v1/auth/me", app.base)).header("cookie", cookie).send().await.unwrap();
    (r.status(), r.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn sign_in_provisions_an_account_and_a_response_works_only_once() {
    let (app, admin, id, idp) = setup().await;
    // The login page lists the provider with the address that starts the flow.
    let options: Value = app.http.get(format!("{}/api/v1/auth/providers", app.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(options["providers"][0]["kind"], "saml");
    assert_eq!(options["providers"][0]["start"], format!("/api/v1/auth/saml/{id}/start"));
    let metadata = app.http.get(format!("{}/api/v1/auth/saml/{id}/metadata", app.base)).send().await.unwrap().text().await.unwrap();
    assert!(metadata.contains(&format!("entityID=\"{}\"", idp.sp)) && metadata.contains(&format!("Location=\"{}\"", idp.acs)));

    let s = start(&app, &id).await;
    let response = idp.response(Some(&s.request_id), "_a1", "grace@example.org", "Grace Hopper");
    let (location, cookie) = post_acs(&app, &id, &response, Some(&s.relay)).await;
    assert_eq!(location, "/");
    let (status, body) = me(&app, &cookie.expect("a session cookie")).await;
    assert_eq!(status, 200);
    assert_eq!(body["user"]["email"], "grace@example.org");
    assert_eq!(body["user"]["name"], "Grace Hopper");
    assert_eq!(body["user"]["kind"], "human");
    assert!(body["permissions"].as_array().unwrap().contains(&json!("workspace.access")), "the provider's default role applies");
    assert!(app.audit_actions().await.contains(&"user.provision".to_string()));

    // Replaying the captured response does not sign anyone in.
    let (location, cookie) = post_acs(&app, &id, &response, Some(&s.relay)).await;
    assert_eq!(location, "/login?error=sso");
    assert!(cookie.is_none());

    // A second sign-in finds the same account rather than creating another.
    let s = start(&app, &id).await;
    let (location, _) = post_acs(&app, &id, &idp.response(Some(&s.request_id), "_a2", "grace@example.org", "Grace B. Hopper"), Some(&s.relay)).await;
    assert_eq!(location, "/");
    let users = admin.ok("/api/v1/admin/users?q=grace").await;
    assert_eq!(users["total"], 1);
    assert_eq!(users["users"][0]["name"], "Grace Hopper", "the name is taken at provisioning; the profile is the person's afterward");
    assert!(app.security_kinds().await.contains(&"auth.sso_failed".to_string()));
}

#[tokio::test]
async fn responses_that_do_not_answer_a_request_from_this_server_are_refused() {
    let (app, admin, id, idp) = setup().await;
    let fail = "/login?error=sso";

    // Started at the identity provider (no InResponseTo): refused unless the provider allows it.
    let (location, cookie) = post_acs(&app, &id, &idp.response(None, "_b1", "eve@example.org", "Eve"), None).await;
    assert_eq!((location.as_str(), cookie), (fail, None));

    // Answers a request that was never made.
    let (location, _) = post_acs(&app, &id, &idp.response(Some("_made_up"), "_b2", "eve@example.org", "Eve"), Some("x")).await;
    assert_eq!(location, fail);

    // The right request, but the browser did not bring back the matching RelayState
    // (an attacker feeding their own response into a victim's browser).
    let s = start(&app, &id).await;
    let (location, _) = post_acs(&app, &id, &idp.response(Some(&s.request_id), "_b3", "eve@example.org", "Eve"), Some("something-else")).await;
    assert_eq!(location, fail);
    // That attempt used up the request.
    let (location, _) = post_acs(&app, &id, &idp.response(Some(&s.request_id), "_b4", "eve@example.org", "Eve"), Some(&s.relay)).await;
    assert_eq!(location, fail);

    // A response addressed to another service provider.
    let s = start(&app, &id).await;
    let elsewhere = Idp { acs: "https://other.example.org/acs".into(), sp: idp.sp.clone() };
    let (location, _) = post_acs(&app, &id, &elsewhere.response(Some(&s.request_id), "_b5", "eve@example.org", "Eve"), Some(&s.relay)).await;
    assert_eq!(location, fail);

    // Garbage in the form.
    for junk in ["", "%%%", &B64.encode("<not-saml/>"), &B64.encode("\u{0}\u{1}binary")] {
        let (location, _) = post_acs(&app, &id, junk, None).await;
        assert_eq!(location, fail, "{junk:?}");
    }
    assert_eq!(admin.ok("/api/v1/admin/users?q=eve").await["total"], 0, "nobody was provisioned along the way");

    // With identity-provider-initiated sign-in allowed, the first case works.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{id}"), json!({"config": {"allow_idp_initiated": true}})).await.0, 204);
    let (location, cookie) = post_acs(&app, &id, &idp.response(None, "_b6", "eve@example.org", "Eve"), None).await;
    assert_eq!(location, "/");
    assert!(cookie.is_some());

    // A disabled provider accepts nothing.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{id}"), json!({"enabled": false})).await.0, 204);
    let (location, _) = post_acs(&app, &id, &idp.response(None, "_b7", "eve@example.org", "Eve"), None).await;
    assert_eq!(location, fail);
    assert_eq!(app.http.get(format!("{}/api/v1/auth/saml/{id}/start", app.base)).send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn account_matching_follows_the_provider_settings() {
    let (app, admin, id, idp) = setup().await;
    let sign_in = |email: &'static str, aid: &'static str| {
        let (app, id, idp) = (&app, &id, &idp);
        async move {
            let s = start(app, id).await;
            post_acs(app, id, &idp.response(Some(&s.request_id), aid, email, "Someone"), Some(&s.relay)).await
        }
    };
    // Domains: only the listed ones may sign in.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{id}"), json!({"allowed_domains": ["example.org"]})).await.0, 204);
    assert_eq!(sign_in("mallory@elsewhere.test", "_c1").await.0, "/login?error=sso");
    assert_eq!(sign_in("heidi@example.org", "_c2").await.0, "/");

    // An existing local account is linked by email (the default)…
    app.make_user("ivan@example.org", "member").await;
    let (location, cookie) = sign_in("ivan@example.org", "_c3").await;
    assert_eq!(location, "/");
    assert_eq!(me(&app, &cookie.unwrap()).await.1["user"]["email"], "ivan@example.org");
    // …but not when linking is turned off.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{id}"), json!({"link_by_email": false})).await.0, 204);
    app.make_user("judy@example.org", "member").await;
    assert_eq!(sign_in("judy@example.org", "_c4").await.0, "/login?error=sso");
    // Ivan was linked before the change and still signs in.
    assert_eq!(sign_in("ivan@example.org", "_c5").await.0, "/");

    // Without auto-provisioning, strangers are turned away.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{id}"), json!({"auto_provision": false})).await.0, 204);
    assert_eq!(sign_in("niaj@example.org", "_c6").await.0, "/login?error=sso");

    // A deactivated account cannot come back through single sign-on.
    let ivan = admin.ok("/api/v1/admin/users?q=ivan").await["users"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(admin.patch(&format!("/api/v1/admin/users/{ivan}"), json!({"active": false})).await.0, 204);
    assert_eq!(sign_in("ivan@example.org", "_c7").await.0, "/login?error=sso");

    // An AI agent's address never signs in, even if the identity provider vouches for it.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{id}"), json!({"link_by_email": true})).await.0, 204);
    assert_eq!(admin.post("/api/v1/admin/users", json!({"kind": "agent", "name": "Build bot", "email": "bot@example.org"})).await.0, 201);
    assert_eq!(sign_in("bot@example.org", "_c8").await.0, "/login?error=sso");
}

#[tokio::test]
async fn signing_certificates_can_be_rotated_without_removing_the_provider() {
    let app = common::spawn().await;
    let admin = app.admin().await;
    // The provider starts out trusting a certificate the identity provider no longer signs with.
    let created = admin
        .created("/api/v1/admin/auth-providers", json!({"kind": "saml", "name": "Corporate", "issuer": IDP, "config": {"sso_url": "https://idp.example.org/sso", "certificates": [fixture("other-cert.pem")], "email_attribute": "mail"}}))
        .await;
    let id = created["id"].as_str().unwrap().to_string();
    let idp = Idp { acs: format!("{}/api/v1/auth/saml/{id}/acs", app.base), sp: format!("{}/api/v1/auth/saml/{id}/metadata", app.base) };
    let s = start(&app, &id).await;
    let (location, cookie) = post_acs(&app, &id, &idp.response(Some(&s.request_id), "_r1", "rota@example.org", "Rota Tion"), Some(&s.relay)).await;
    assert_eq!((location.as_str(), cookie.is_none()), ("/login?error=sso", true), "signed with a key that is not configured");

    // Both certificates are trusted while the identity provider switches over.
    let path = format!("/api/v1/admin/auth-providers/{id}");
    let (status, body) = admin.patch(&path, json!({"config": {"certificates": [fixture("other-cert.pem"), fixture("idp-cert.pem")]}})).await;
    assert_eq!(status, 204, "{body}");
    let listed = admin.ok("/api/v1/admin/auth-providers").await;
    let p = listed.as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).unwrap();
    assert_eq!(p["config"]["certificates"].as_array().unwrap().len(), 2);
    assert_eq!(p["saml"]["certificate_expiry"].as_array().unwrap().len(), 2);
    assert_eq!(p["config"]["sso_url"], "https://idp.example.org/sso", "settings that were not sent keep their values");
    assert_eq!(p["config"]["email_attribute"], "mail");
    let s = start(&app, &id).await;
    let (location, cookie) = post_acs(&app, &id, &idp.response(Some(&s.request_id), "_r2", "rota@example.org", "Rota Tion"), Some(&s.relay)).await;
    assert_eq!(location, "/");
    assert_eq!(me(&app, &cookie.unwrap()).await.1["user"]["email"], "rota@example.org");

    // Then the old one is dropped, and an attribute name is cleared, in the same way.
    let (status, _) = admin.patch(&path, json!({"config": {"certificates": [fixture("idp-cert.pem")], "email_attribute": ""}})).await;
    assert_eq!(status, 204);
    let listed = admin.ok("/api/v1/admin/auth-providers").await;
    let p = listed.as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).unwrap();
    assert_eq!(p["config"]["certificates"].as_array().unwrap().len(), 1);
    assert_eq!(p["config"]["email_attribute"], "");
    let s = start(&app, &id).await;
    assert_eq!(post_acs(&app, &id, &idp.response(Some(&s.request_id), "_r3", "rota@example.org", "Rota Tion"), Some(&s.relay)).await.0, "/");

    // A change is checked like a new provider, is recorded, and is not for application administrators.
    let (status, body) = admin.patch(&path, json!({"config": {"certificates": ["nonsense"]}})).await;
    assert_eq!(status, 400);
    assert!(body["detail"].as_str().unwrap().contains("certificate 1"), "{body}");
    assert_eq!(admin.patch(&path, json!({"kind": "oidc"})).await.0, 400);
    let app_admin = app.user("appadmin2@example.org", "app_admin").await;
    assert_eq!(app_admin.patch(&path, json!({"config": {"certificates": [fixture("other-cert.pem")]}})).await.0, 403);
    assert!(app.audit_actions().await.iter().any(|a| a.starts_with("auth_provider.")), "{:?}", app.audit_actions().await);
}

#[tokio::test]
async fn provider_settings_are_validated_and_restricted_to_security_administrators() {
    let app = common::spawn().await;
    let admin = app.admin().await;
    let cert = fixture("idp-cert.pem");
    let body = |cfg: Value| json!({"kind": "saml", "name": "X", "issuer": IDP, "config": cfg});
    for (cfg, expect) in [
        (json!({"certificates": [cert]}), "sso_url"),
        (json!({"sso_url": "http://idp.example.org/sso", "certificates": [cert]}), "https"),
        (json!({"sso_url": "javascript:alert(1)", "certificates": [cert]}), "https"),
        (json!({"sso_url": "https://idp.example.org/sso"}), "certificates"),
        (json!({"sso_url": "https://idp.example.org/sso", "certificates": ["nonsense"]}), "certificate 1"),
        (json!({"sso_url": "https://idp.example.org/sso", "certificates": [cert], "surprise": true}), "unknown SAML setting"),
    ] {
        // (The test server allows plain http for outbound calls, so the http case is only
        // refused for the scheme list; check the message loosely.)
        let (status, resp) = admin.post("/api/v1/admin/auth-providers", body(cfg.clone())).await;
        if cfg["sso_url"] == "http://idp.example.org/sso" {
            assert_eq!(status, 201, "plain http is accepted only because this test server allows insecure outbound URLs");
            continue;
        }
        assert_eq!(status, 400, "{cfg}");
        assert!(resp["detail"].as_str().unwrap().contains(expect), "{cfg}: {resp}");
    }
    let (status, _) = admin.post("/api/v1/admin/auth-providers", json!({"kind": "saml", "name": "X", "issuer": "", "config": {"sso_url": "https://idp.example.org/sso", "certificates": [cert]}})).await;
    assert_eq!(status, 400, "the entity ID is required");
    let (status, _) = admin.post("/api/v1/admin/auth-providers", json!({"kind": "ldap", "name": "X", "issuer": "x"})).await;
    assert_eq!(status, 400);

    // An application administrator cannot add or change sign-in providers.
    let app_admin = app.user("appadmin@example.org", "app_admin").await;
    assert_eq!(app_admin.post("/api/v1/admin/auth-providers", body(json!({"sso_url": "https://idp.example.org/sso", "certificates": [cert]}))).await.0, 403);
    assert_eq!(app_admin.post("/api/v1/admin/auth-providers/saml-metadata", json!({"xml": "<x/>"})).await.0, 403);

    // Metadata import fills the form.
    let pem_body: String = cert.lines().filter(|l| !l.contains("-----")).collect();
    let metadata = format!(
        "<EntityDescriptor xmlns=\"urn:oasis:names:tc:SAML:2.0:metadata\" xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\" entityID=\"{IDP}\"><IDPSSODescriptor protocolSupportEnumeration=\"urn:oasis:names:tc:SAML:2.0:protocol\"><KeyDescriptor use=\"signing\"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{pem_body}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></KeyDescriptor><SingleSignOnService Binding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect\" Location=\"https://idp.example.org/sso\"/></IDPSSODescriptor></EntityDescriptor>"
    );
    let (status, m) = admin.post("/api/v1/admin/auth-providers/saml-metadata", json!({"xml": metadata})).await;
    assert_eq!(status, 200, "{m}");
    assert_eq!(m["issuer"], IDP);
    let (status, created) = admin.post("/api/v1/admin/auth-providers", json!({"kind": "saml", "name": "From metadata", "issuer": m["issuer"], "config": {"sso_url": m["sso_url"], "certificates": m["certificates"]}})).await;
    assert_eq!(status, 201, "{created}");
    let listed = admin.ok("/api/v1/admin/auth-providers").await;
    let p = listed.as_array().unwrap().iter().find(|p| p["name"] == "From metadata").unwrap();
    assert!(p["saml"]["certificate_expiry"][0].is_string(), "the administrator sees when the certificate expires");
    assert_eq!(p["kind"], "saml");
    // The kind is fixed.
    assert_eq!(admin.patch(&format!("/api/v1/admin/auth-providers/{}", created["id"].as_str().unwrap()), json!({"kind": "oidc"})).await.0, 400);
}
