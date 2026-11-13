//! Single sign-on with SAML 2.0 (Web Browser SSO profile, service provider side).
//!
//! Flow: `start` redirects the browser to the identity provider with an `AuthnRequest`
//! (HTTP-Redirect binding); the provider posts a signed `Response` back to `acs` (HTTP-POST
//! binding). Works with Microsoft Entra ID, ADFS, Okta, Ping, Keycloak, Shibboleth, Google
//! Workspace and other standards-compliant providers.
//!
//! What is checked before anyone is signed in:
//!
//! * the XML has no DTD (so no entity tricks) and is of modest size;
//! * the assertion that is used is covered by a valid XML signature made with one of the
//!   certificates the administrator configured. Keys inside the message are ignored;
//! * the signature is an enveloped signature over exactly the element that carries it, with
//!   exclusive canonicalization and SHA-256 or stronger. SHA-1 and unusual transforms are refused;
//! * the signed element's ID occurs once in the document, and only data from inside the
//!   verified element is read (defeats signature wrapping);
//! * issuer, audience, recipient, destination, validity window and `InResponseTo` match this
//!   service provider and a sign-in that this server started;
//! * each assertion is accepted once.
//!
//! Not supported: encrypted assertions (use TLS, which every deployment already has), signed
//! `AuthnRequest`s, and single logout. Sessions end by their own timeouts.

use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, ReqInfo};
use crate::oidc::{self, SsoIdentity, SsoProvider};
use crate::{audit, auth, crypto, AppState, St};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use roxmltree::{Document, Node, ParsingOptions};
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha384, Sha512};
use sqlx::Row;
use std::collections::HashMap;
use std::io::Write as _;
use uuid::Uuid;

const NS_SAMLP: &str = "urn:oasis:names:tc:SAML:2.0:protocol";
const NS_SAML: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
const NS_MD: &str = "urn:oasis:names:tc:SAML:2.0:metadata";
const NS_DS: &str = "http://www.w3.org/2000/09/xmldsig#";
const EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
const ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
const STATUS_SUCCESS: &str = "urn:oasis:names:tc:SAML:2.0:status:Success";
const BEARER: &str = "urn:oasis:names:tc:SAML:2.0:cm:bearer";
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
/// Allowed difference between this server's clock and the identity provider's.
const SKEW_SECONDS: i64 = 120;

// ───────────────────────── XML helpers ─────────────────────────

/// Real SAML messages nest about a dozen levels.
const MAX_DEPTH: usize = 48;
/// The busiest element of a real message carries about a dozen attributes and namespace
/// declarations.
const MAX_ATTRIBUTES: usize = 64;

/// Whether elements in `xml` nest at most `limit` deep, judged from the text alone.
///
/// The XML parser recurses once per level of nesting, so a message of a few hundred
/// kilobytes of `<a><a><a>…` would exhaust the stack and take the whole process down. This
/// endpoint is reachable without signing in, so the depth is measured here first, with a
/// scanner that does not recurse. It follows the same left-to-right rules as the parser for
/// comments, CDATA, processing instructions and quoted attribute values; wherever the text
/// is malformed it stops and lets the parser report the error, which the parser reaches
/// before it can go any deeper than what was already measured.
pub fn nesting_within(xml: &str, limit: usize) -> bool {
    shape_within(xml, limit, usize::MAX)
}

/// [`nesting_within`], and no element with more than `attributes` attributes. The parser
/// compares each attribute of an element with all those before it, so one element with a
/// hundred thousand of them costs it billions of comparisons: seconds of processor time per
/// request, from anyone. Counted here in one pass, also in a tag that never ends.
pub fn shape_within(xml: &str, limit: usize, attributes: usize) -> bool {
    let b = xml.as_bytes();
    let (mut i, mut depth) = (0usize, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        let skip_to = |terminator: &[u8]| rest.windows(terminator.len()).position(|w| w == terminator).map(|p| p + terminator.len());
        let advance = if rest.starts_with(b"<!--") {
            skip_to(b"-->")
        } else if rest.starts_with(b"<![CDATA[") {
            skip_to(b"]]>")
        } else if rest.starts_with(b"<?") {
            skip_to(b"?>")
        } else if rest.starts_with(b"<!") {
            // A DTD or other declaration: the parser refuses it on sight.
            return true;
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            skip_to(b">")
        } else {
            // A start tag ends at the first `>` outside a quoted attribute value.
            let mut quote = 0u8;
            let mut end = None;
            let mut values = 0usize;
            for (j, c) in rest.iter().enumerate().skip(1) {
                if quote != 0 {
                    if *c == quote {
                        quote = 0;
                        values += 1;
                        if values > attributes {
                            return false;
                        }
                    }
                } else if *c == b'"' || *c == b'\'' {
                    quote = *c;
                } else if *c == b'>' {
                    end = Some(j);
                    break;
                }
            }
            if let Some(e) = end {
                if rest[e - 1] != b'/' {
                    depth += 1;
                    if depth > limit {
                        return false;
                    }
                }
            }
            end.map(|e| e + 1)
        };
        match advance {
            Some(n) => i += n,
            None => return true,
        }
    }
    true
}

fn parse(xml: &str) -> Result<Document<'_>, String> {
    if !shape_within(xml, MAX_DEPTH, MAX_ATTRIBUTES) {
        return Err("the message is not acceptable XML: elements are nested too deeply or carry too many attributes".into());
    }
    let opt = ParsingOptions { allow_dtd: false, nodes_limit: 20_000 };
    Document::parse_with_options(xml, opt).map_err(|e| format!("the message is not acceptable XML: {e}"))
}

fn is(n: Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(ns) && n.tag_name().name() == name
}

fn children<'a, 'i>(n: Node<'a, 'i>, ns: &'static str, name: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children().filter(move |c| is(*c, ns, name))
}

fn child<'a, 'i>(n: Node<'a, 'i>, ns: &'static str, name: &'static str) -> Option<Node<'a, 'i>> {
    children(n, ns, name).next()
}

/// All text inside an element. `Node::text` would return only the part before a comment,
/// which is how `admin@example.org<!---->.attacker.test` has fooled other implementations.
fn full_text(n: Node) -> String {
    n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect::<String>()
}

fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn element_qname<'s>(src: &'s str, n: Node) -> &'s str {
    let rest = &src[n.range().start + 1..];
    &rest[..rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len())]
}

fn attribute_qname<'s>(src: &'s str, a: &roxmltree::Attribute) -> &'s str {
    let rest = &src[a.range().start..];
    &rest[..rest.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(rest.len())]
}

// ───────────────────────── exclusive canonicalization ─────────────────────────

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

fn write_element(src: &str, n: Node, skip: Option<Node>, inclusive: &[String], rendered: &mut Vec<(String, String)>, out: &mut String) {
    let qname = element_qname(src, n);
    let own_prefix = qname.split_once(':').map(|p| p.0).unwrap_or("");
    // (namespace URI, local name, name as written, value)
    let mut attrs: Vec<(&str, &str, &str, &str)> = n.attributes().map(|a| (a.namespace().unwrap_or(""), a.name(), attribute_qname(src, &a), a.value())).collect();
    attrs.sort();
    // Prefixes this element visibly uses: its own, those of its attributes, and any the
    // signer asked to be treated inclusively.
    let mut wanted: Vec<(&str, Option<&str>)> = vec![(own_prefix, n.tag_name().namespace())];
    for a in &attrs {
        if let Some((p, _)) = a.2.split_once(':') {
            wanted.push((p, Some(a.0)));
        }
    }
    for p in inclusive {
        let key = if p == "#default" { "" } else { p.as_str() };
        if let Some(uri) = n.lookup_namespace_uri(if key.is_empty() { None } else { Some(key) }) {
            wanted.push((key, Some(uri)));
        }
    }
    wanted.sort();
    wanted.dedup();
    let mark = rendered.len();
    let mut decls: Vec<(&str, &str)> = vec![];
    for (prefix, uri) in wanted {
        if prefix == "xml" {
            continue;
        }
        let current = rendered.iter().rev().find(|r| r.0 == prefix).map(|r| r.1.as_str());
        match uri.filter(|u| !u.is_empty()) {
            Some(u) if current == Some(u) => {}
            Some(u) => {
                decls.push((prefix, u));
                rendered.push((prefix.to_string(), u.to_string()));
            }
            // An element in no namespace under an ancestor that declared a default one.
            None if prefix.is_empty() && current.is_some_and(|c| !c.is_empty()) => {
                decls.push(("", ""));
                rendered.push((String::new(), String::new()));
            }
            None => {}
        }
    }
    out.push('<');
    out.push_str(qname);
    for (prefix, uri) in decls {
        if prefix.is_empty() {
            out.push_str(" xmlns=\"");
        } else {
            out.push_str(" xmlns:");
            out.push_str(prefix);
            out.push_str("=\"");
        }
        escape_attr(uri, out);
        out.push('"');
    }
    for (_, _, name, value) in attrs {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape_attr(value, out);
        out.push('"');
    }
    out.push('>');
    for c in n.children() {
        if Some(c) == skip {
            continue;
        }
        if c.is_element() {
            write_element(src, c, skip, inclusive, rendered, out);
        } else if c.is_text() {
            escape_text(c.text().unwrap_or_default(), out);
        } else if let Some(pi) = c.pi() {
            out.push_str("<?");
            out.push_str(pi.target);
            if let Some(v) = pi.value.filter(|v| !v.is_empty()) {
                out.push(' ');
                out.push_str(v);
            }
            out.push_str("?>");
        }
        // Comments are omitted (canonicalization without comments).
    }
    out.push_str("</");
    out.push_str(qname);
    out.push('>');
    rendered.truncate(mark);
}

/// Exclusive XML Canonicalization 1.0 (without comments) of the subtree at `node`, leaving
/// out the subtree `skip`. `src` is the text the document was parsed from.
pub fn canonicalize(src: &str, node: Node, skip: Option<Node>, inclusive_prefixes: &[String]) -> String {
    let mut out = String::new();
    write_element(src, node, skip, inclusive_prefixes, &mut Vec::new(), &mut out);
    out
}

fn prefix_list(parent: Node) -> Vec<String> {
    parent.children().find(|c| is(*c, EXC_C14N, "InclusiveNamespaces")).and_then(|c| c.attribute("PrefixList")).map(|l| l.split_whitespace().map(str::to_string).collect()).unwrap_or_default()
}

// ───────────────────────── signatures ─────────────────────────

fn hash(alg: &str, data: &[u8]) -> Option<(Vec<u8>, Pkcs1v15Sign)> {
    Some(match alg {
        "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256" | "http://www.w3.org/2001/04/xmlenc#sha256" => (Sha256::digest(data).to_vec(), Pkcs1v15Sign::new::<Sha256>()),
        "http://www.w3.org/2001/04/xmldsig-more#rsa-sha384" | "http://www.w3.org/2001/04/xmldsig-more#sha384" => (Sha384::digest(data).to_vec(), Pkcs1v15Sign::new::<Sha384>()),
        "http://www.w3.org/2001/04/xmldsig-more#rsa-sha512" | "http://www.w3.org/2001/04/xmlenc#sha512" => (Sha512::digest(data).to_vec(), Pkcs1v15Sign::new::<Sha512>()),
        _ => return None,
    })
}

/// Verify the enveloped signature carried by `signed` (a `Response` or an `Assertion`).
/// On success, everything inside `signed` except the signature itself is authentic.
pub fn verify_signature(src: &str, doc: &Document, signed: Node, keys: &[RsaPublicKey]) -> Result<(), String> {
    let sigs: Vec<Node> = children(signed, NS_DS, "Signature").collect();
    let [sig] = sigs.as_slice() else { return Err("expected exactly one signature on the signed element".into()) };
    let info = child(*sig, NS_DS, "SignedInfo").ok_or("the signature has no SignedInfo")?;
    let c14n = child(info, NS_DS, "CanonicalizationMethod").ok_or("the signature names no canonicalization method")?;
    if c14n.attribute("Algorithm") != Some(EXC_C14N) {
        return Err("only exclusive XML canonicalization is accepted".into());
    }
    let method = child(info, NS_DS, "SignatureMethod").and_then(|m| m.attribute("Algorithm")).ok_or("the signature names no signature method")?;
    if !method.contains("#rsa-sha") {
        return Err("only RSA signatures with SHA-256, SHA-384 or SHA-512 are accepted".into());
    }
    let refs: Vec<Node> = children(info, NS_DS, "Reference").collect();
    let [reference] = refs.as_slice() else { return Err("the signature must reference exactly one element".into()) };
    let id = signed.attribute("ID").filter(|i| !i.is_empty()).ok_or("the signed element has no ID")?;
    if reference.attribute("URI") != Some(format!("#{id}").as_str()) {
        return Err("the signature does not cover the element that carries it".into());
    }
    // An ID that occurs twice lets a forged element shadow the signed one.
    if doc.descendants().filter(|d| d.is_element() && d.attributes().any(|a| a.name().eq_ignore_ascii_case("id") && a.value() == id)).count() != 1 {
        return Err("the signed element's ID is not unique in the message".into());
    }
    let mut enveloped = false;
    let mut prefixes = vec![];
    if let Some(transforms) = child(*reference, NS_DS, "Transforms") {
        for t in transforms.children().filter(Node::is_element) {
            match t.attribute("Algorithm") {
                Some(ENVELOPED) if is(t, NS_DS, "Transform") => enveloped = true,
                Some(EXC_C14N) if is(t, NS_DS, "Transform") => prefixes = prefix_list(t),
                _ => return Err("the signature uses a transform that is not accepted".into()),
            }
        }
    }
    if !enveloped {
        return Err("the signature must be an enveloped signature".into());
    }
    let digest_alg = child(*reference, NS_DS, "DigestMethod").and_then(|m| m.attribute("Algorithm")).ok_or("the signature names no digest method")?;
    let canonical = canonicalize(src, signed, Some(*sig), &prefixes);
    let (digest, _) = hash(digest_alg, canonical.as_bytes()).ok_or("only SHA-256, SHA-384 and SHA-512 digests are accepted")?;
    let claimed = child(*reference, NS_DS, "DigestValue").and_then(|d| B64.decode(squeeze(&full_text(d))).ok()).ok_or("the digest value is missing or unreadable")?;
    if !crypto::ct_eq(&digest, &claimed) {
        return Err("the content does not match its signed digest (it was changed after signing)".into());
    }
    let signed_info = canonicalize(src, info, None, &prefix_list(c14n));
    let value = child(*sig, NS_DS, "SignatureValue").and_then(|d| B64.decode(squeeze(&full_text(d))).ok()).ok_or("the signature value is missing or unreadable")?;
    let (hashed, _) = hash(method, signed_info.as_bytes()).ok_or("only RSA signatures with SHA-256, SHA-384 or SHA-512 are accepted")?;
    for key in keys {
        let (_, scheme) = hash(method, b"").expect("checked above");
        if key.verify(scheme, &hashed, &value).is_ok() {
            return Ok(());
        }
    }
    Err("the signature was not made with any of the configured certificates".into())
}

/// Public keys from certificates given as PEM, or as the bare base64 found in metadata.
pub fn keys_from_certificates(certs: &[String]) -> Result<Vec<(RsaPublicKey, Option<DateTime<Utc>>)>, String> {
    use x509_cert::der::{Decode, Encode};
    let mut out = vec![];
    for (i, text) in certs.iter().enumerate() {
        let body: String = text.lines().filter(|l| !l.contains("-----")).collect();
        let der = B64.decode(squeeze(&body)).map_err(|_| format!("certificate {} is not valid base64 or PEM", i + 1))?;
        let cert = x509_cert::Certificate::from_der(&der).map_err(|e| format!("certificate {} could not be read: {e}", i + 1))?;
        let spki = cert.tbs_certificate.subject_public_key_info.to_der().map_err(|e| format!("certificate {}: {e}", i + 1))?;
        let key = RsaPublicKey::from_public_key_der(&spki).map_err(|_| format!("certificate {} does not hold an RSA key; only RSA signing certificates are supported", i + 1))?;
        if rsa::traits::PublicKeyParts::size(&key) < 256 {
            return Err(format!("certificate {} uses an RSA key shorter than 2048 bits", i + 1));
        }
        let expires = DateTime::<Utc>::from_timestamp(cert.tbs_certificate.validity.not_after.to_unix_duration().as_secs() as i64, 0);
        out.push((key, expires));
    }
    Ok(out)
}

// ───────────────────────── reading a response ─────────────────────────

pub struct Expect<'a> {
    pub idp_entity_id: &'a str,
    pub sp_entity_id: &'a str,
    pub acs_url: &'a str,
    pub now: DateTime<Utc>,
    pub keys: &'a [RsaPublicKey],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    pub id: String,
    pub name_id: String,
    pub name_id_format: String,
    pub attributes: HashMap<String, Vec<String>>,
    /// The request this answers; `None` when the identity provider started the sign-in.
    pub in_response_to: Option<String>,
    pub expires: DateTime<Utc>,
}

fn time(v: Option<&str>, what: &str) -> Result<Option<DateTime<Utc>>, String> {
    match v {
        None => Ok(None),
        Some(s) => DateTime::parse_from_rfc3339(s).map(|t| Some(t.with_timezone(&Utc))).map_err(|_| format!("{what} is not a valid time")),
    }
}

/// Validate a SAML `Response` and return the assertion it carries.
pub fn read_response(xml: &str, e: &Expect) -> Result<Assertion, String> {
    if xml.len() > MAX_RESPONSE_BYTES {
        return Err("the response is too large".into());
    }
    let doc = parse(xml)?;
    let response = doc.root_element();
    if !is(response, NS_SAMLP, "Response") {
        return Err("the message is not a SAML response".into());
    }
    let status = child(response, NS_SAMLP, "Status").and_then(|s| child(s, NS_SAMLP, "StatusCode")).and_then(|c| c.attribute("Value")).unwrap_or_default();
    if status != STATUS_SUCCESS {
        return Err(format!("the identity provider reported {}", status.rsplit(':').next().unwrap_or("a failure")));
    }
    if children(response, NS_SAML, "EncryptedAssertion").next().is_some() {
        return Err("the assertion is encrypted, which is not supported; turn assertion encryption off at the identity provider".into());
    }
    let assertions: Vec<Node> = children(response, NS_SAML, "Assertion").collect();
    let [assertion] = assertions.as_slice() else { return Err("expected exactly one assertion".into()) };
    let assertion = *assertion;
    // The assertion must be covered by a signature: its own, or the enclosing response's.
    // Whichever verifies, only this assertion (a direct child of the response) is read below.
    let on_assertion = child(assertion, NS_DS, "Signature").is_some();
    let on_response = child(response, NS_DS, "Signature").is_some();
    if on_assertion {
        verify_signature(xml, &doc, assertion, e.keys)?;
    }
    if on_response {
        verify_signature(xml, &doc, response, e.keys)?;
    }
    if !on_assertion && !on_response {
        return Err("neither the response nor the assertion is signed".into());
    }

    if let Some(dest) = response.attribute("Destination") {
        if dest != e.acs_url {
            return Err("the response was addressed to a different service".into());
        }
    }
    let issuer = child(assertion, NS_SAML, "Issuer").map(full_text).unwrap_or_default();
    if issuer.trim() != e.idp_entity_id {
        return Err("the assertion was issued by a different identity provider".into());
    }
    if let Some(i) = child(response, NS_SAML, "Issuer") {
        if full_text(i).trim() != e.idp_entity_id {
            return Err("the response was issued by a different identity provider".into());
        }
    }
    let skew = Duration::seconds(SKEW_SECONDS);
    let conditions = child(assertion, NS_SAML, "Conditions").ok_or("the assertion has no conditions")?;
    if time(conditions.attribute("NotBefore"), "NotBefore")?.is_some_and(|t| e.now + skew < t) {
        return Err("the assertion is not valid yet (check both clocks)".into());
    }
    let cond_end = time(conditions.attribute("NotOnOrAfter"), "NotOnOrAfter")?;
    if cond_end.is_some_and(|t| e.now - skew >= t) {
        return Err("the assertion has expired".into());
    }
    let audiences: Vec<String> = children(conditions, NS_SAML, "AudienceRestriction").flat_map(|r| children(r, NS_SAML, "Audience")).map(|a| full_text(a).trim().to_string()).collect();
    if !audiences.iter().any(|a| a == e.sp_entity_id) {
        return Err("the assertion is meant for a different service (audience mismatch)".into());
    }
    let subject = child(assertion, NS_SAML, "Subject").ok_or("the assertion has no subject")?;
    let name_id_node = child(subject, NS_SAML, "NameID").ok_or("the assertion names no user (NameID)")?;
    let name_id = full_text(name_id_node).trim().to_string();
    if name_id.is_empty() || name_id.len() > 512 {
        return Err("the assertion names no user (NameID)".into());
    }
    // Bearer confirmation ties the assertion to this endpoint and to our request.
    let mut confirmed: Option<(Option<String>, DateTime<Utc>)> = None;
    for c in children(subject, NS_SAML, "SubjectConfirmation").filter(|c| c.attribute("Method") == Some(BEARER)) {
        let Some(data) = child(c, NS_SAML, "SubjectConfirmationData") else { continue };
        let Some(end) = time(data.attribute("NotOnOrAfter"), "NotOnOrAfter")? else { continue };
        if data.attribute("Recipient") == Some(e.acs_url) && e.now - skew < end && data.attribute("NotBefore").is_none() {
            confirmed = Some((data.attribute("InResponseTo").map(str::to_string), end));
            break;
        }
    }
    let (in_response_to, confirm_end) = confirmed.ok_or("the assertion was not issued for this sign-in endpoint, or has expired")?;
    if response.attribute("InResponseTo").map(str::to_string) != in_response_to && response.attribute("InResponseTo").is_some() {
        return Err("the response and the assertion answer different requests".into());
    }
    let mut attributes: HashMap<String, Vec<String>> = HashMap::new();
    for a in children(assertion, NS_SAML, "AttributeStatement").flat_map(|s| children(s, NS_SAML, "Attribute")) {
        let values: Vec<String> = children(a, NS_SAML, "AttributeValue").map(|v| full_text(v).trim().to_string()).filter(|v| !v.is_empty() && v.len() <= 1024).take(50).collect();
        for name in [a.attribute("Name"), a.attribute("FriendlyName")].into_iter().flatten() {
            attributes.entry(name.to_string()).or_default().extend(values.iter().cloned());
        }
    }
    Ok(Assertion {
        id: assertion.attribute("ID").unwrap_or_default().to_string(),
        name_id,
        name_id_format: name_id_node.attribute("Format").unwrap_or_default().to_string(),
        attributes,
        in_response_to,
        expires: cond_end.map(|c| c.min(confirm_end)).unwrap_or(confirm_end),
    })
}

const EMAIL_ATTRIBUTES: &[&str] = &["email", "mail", "emailAddress", "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress", "urn:oid:0.9.2342.19200300.100.1.3", "User.email"];
const NAME_ATTRIBUTES: &[&str] = &["displayName", "name", "http://schemas.microsoft.com/identity/claims/displayname", "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/name", "urn:oid:2.16.840.1.113730.3.1.241", "cn", "urn:oid:2.5.4.3"];
const GIVEN: &[&str] = &["givenName", "firstName", "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/givenname", "urn:oid:2.5.4.42"];
const SURNAME: &[&str] = &["sn", "surname", "lastName", "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/surname", "urn:oid:2.5.4.4"];

/// Pick the email address and display name out of an assertion.
pub fn identity(a: &Assertion, email_attribute: &str, name_attribute: &str) -> SsoIdentity {
    let first = |names: &[&str]| names.iter().find_map(|n| a.attributes.get(*n).and_then(|v| v.first())).cloned();
    let email = if email_attribute.is_empty() { first(EMAIL_ATTRIBUTES) } else { first(&[email_attribute]) }.or_else(|| a.name_id.contains('@').then(|| a.name_id.clone()));
    let name = if name_attribute.is_empty() { first(NAME_ATTRIBUTES) } else { first(&[name_attribute]) }.or_else(|| match (first(GIVEN), first(SURNAME)) {
        (Some(g), Some(s)) => Some(format!("{g} {s}")),
        (g, s) => g.or(s),
    });
    SsoIdentity { subject: a.name_id.clone(), email, email_unverified: false, name }
}

// ───────────────────────── provider settings ─────────────────────────

struct Provider {
    sso: SsoProvider,
    idp_entity_id: String,
    sso_url: String,
    certificates: Vec<String>,
    sp_entity_id: String,
    acs_url: String,
    email_attribute: String,
    name_attribute: String,
    allow_idp_initiated: bool,
}

fn base_url(st: &AppState, headers: &HeaderMap) -> String {
    if !st.cfg.server.public_url.is_empty() {
        return st.cfg.server.public_url.trim_end_matches('/').to_string();
    }
    format!("http://{}", headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost"))
}

pub fn acs_url(base: &str, id: Uuid) -> String {
    format!("{base}/api/v1/auth/saml/{id}/acs")
}

pub fn default_entity_id(base: &str, id: Uuid) -> String {
    format!("{base}/api/v1/auth/saml/{id}/metadata")
}

async fn provider(st: &AppState, headers: &HeaderMap, id: Uuid) -> AppResult<Provider> {
    let r = sqlx::query("SELECT id, name, issuer, config, auto_provision, link_by_email, allowed_domains, default_role FROM auth_providers WHERE id = $1 AND enabled AND kind = 'saml'").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let cfg: Value = r.get("config");
    let base = base_url(st, headers);
    let text = |k: &str| cfg[k].as_str().unwrap_or_default().to_string();
    Ok(Provider {
        sso: SsoProvider { id, name: r.get("name"), auto_provision: r.get("auto_provision"), link_by_email: r.get("link_by_email"), allowed_domains: r.get("allowed_domains"), default_role: r.get("default_role") },
        idp_entity_id: r.get("issuer"),
        sso_url: text("sso_url"),
        certificates: cfg["certificates"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        sp_entity_id: Some(text("sp_entity_id")).filter(|s| !s.is_empty()).unwrap_or_else(|| default_entity_id(&base, id)),
        acs_url: acs_url(&base, id),
        email_attribute: text("email_attribute"),
        name_attribute: text("name_attribute"),
        allow_idp_initiated: cfg["allow_idp_initiated"] == true,
    })
}

/// Check SAML provider settings before they are stored and return them normalized.
/// Used by the administration API.
pub fn check_config(st: &AppState, issuer: &str, cfg: &Value) -> AppResult<Value> {
    let o = cfg.as_object().ok_or_else(|| bad("config must be an object"))?;
    if let Some(k) = o.keys().find(|k| !["sso_url", "certificates", "sp_entity_id", "email_attribute", "name_attribute", "allow_idp_initiated"].contains(&k.as_str())) {
        return Err(bad(format!("unknown SAML setting {k:?}")));
    }
    if issuer.trim().is_empty() {
        return Err(bad("the identity provider's entity ID (issuer) is required"));
    }
    let sso_url = o.get("sso_url").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let parsed = url::Url::parse(sso_url).map_err(|_| bad("sso_url must be the identity provider's sign-on address"))?;
    if !(parsed.scheme() == "https" || (st.cfg.security.allow_insecure_outbound && parsed.scheme() == "http")) {
        return Err(bad("sso_url must use https"));
    }
    let certs: Vec<String> = o.get("certificates").map(|c| serde_json::from_value(c.clone())).transpose().map_err(|_| bad("certificates must be a list of PEM texts"))?.unwrap_or_default();
    let certs: Vec<String> = certs.into_iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    if certs.is_empty() || certs.len() > 6 {
        return Err(bad("give one to six signing certificates of the identity provider"));
    }
    keys_from_certificates(&certs).map_err(bad)?;
    let short = |k: &str| -> AppResult<String> {
        let v = o.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if v.len() > 300 || v.chars().any(char::is_control) {
            return Err(bad(format!("{k} is not valid")));
        }
        Ok(v)
    };
    Ok(json!({"sso_url": sso_url, "certificates": certs, "sp_entity_id": short("sp_entity_id")?, "email_attribute": short("email_attribute")?,
              "name_attribute": short("name_attribute")?, "allow_idp_initiated": o.get("allow_idp_initiated").and_then(Value::as_bool).unwrap_or(false)}))
}

/// What an administrator needs to see about a stored SAML provider.
pub fn describe(st: &AppState, id: Uuid, cfg: &Value) -> Value {
    let base = st.cfg.server.public_url.trim_end_matches('/');
    let certs: Vec<String> = cfg["certificates"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let expiry: Vec<Value> = keys_from_certificates(&certs).map(|k| k.into_iter().map(|(_, e)| json!(e)).collect()).unwrap_or_default();
    json!({
        "acs_url": acs_url(base, id),
        "sp_entity_id": cfg["sp_entity_id"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| default_entity_id(base, id)),
        "metadata_url": default_entity_id(base, id),
        "certificate_expiry": expiry,
    })
}

/// Read the entity ID, sign-on address and signing certificates out of identity provider metadata.
/// Identity provider metadata is a small document; federation aggregates are not accepted.
const MAX_METADATA: usize = 2 * 1024 * 1024;

pub fn read_metadata(xml: &str) -> Result<Value, String> {
    if xml.len() > MAX_METADATA {
        return Err("the metadata is too large".into());
    }
    let doc = parse(xml)?;
    let entity = doc.descendants().find(|n| is(*n, NS_MD, "EntityDescriptor") && children(*n, NS_MD, "IDPSSODescriptor").next().is_some()).ok_or("no identity provider was found in the metadata")?;
    let idp = child(entity, NS_MD, "IDPSSODescriptor").ok_or("no identity provider was found in the metadata")?;
    let sso = children(idp, NS_MD, "SingleSignOnService").find(|s| s.attribute("Binding") == Some("urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect")).and_then(|s| s.attribute("Location")).ok_or("the identity provider offers no HTTP-Redirect sign-on address")?;
    let certificates: Vec<String> = children(idp, NS_MD, "KeyDescriptor").filter(|k| k.attribute("use") != Some("encryption")).flat_map(|k| k.descendants().filter(|d| is(*d, NS_DS, "X509Certificate")).map(|d| squeeze(&full_text(d))).collect::<Vec<_>>()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    Ok(json!({"issuer": entity.attribute("entityID").unwrap_or_default(), "sso_url": sso, "certificates": certificates}))
}

// ───────────────────────── endpoints ─────────────────────────

fn redirect(to: &str) -> Response {
    let mut r = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(to) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r
}

fn xml_escape(s: &str) -> String {
    let mut out = String::new();
    escape_attr(s, &mut out);
    out
}

/// The `AuthnRequest` for a sign-in, as XML.
pub fn authn_request(id: &str, issued: DateTime<Utc>, destination: &str, acs: &str, sp_entity_id: &str) -> String {
    format!(
        "<samlp:AuthnRequest xmlns:samlp=\"{NS_SAMLP}\" xmlns:saml=\"{NS_SAML}\" ID=\"{}\" Version=\"2.0\" IssueInstant=\"{}\" Destination=\"{}\" AssertionConsumerServiceURL=\"{}\" ProtocolBinding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\"><saml:Issuer>{}</saml:Issuer><samlp:NameIDPolicy AllowCreate=\"true\"/></samlp:AuthnRequest>",
        xml_escape(id),
        issued.format("%Y-%m-%dT%H:%M:%SZ"),
        xml_escape(destination),
        xml_escape(acs),
        xml_escape(sp_entity_id)
    )
}

/// DEFLATE and base64, as the HTTP-Redirect binding requires.
pub fn deflate_b64(xml: &str) -> String {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(xml.as_bytes());
    B64.encode(enc.finish().unwrap_or_default())
}

async fn start(State(st): State<St>, headers: HeaderMap, Path(id): Path<Uuid>) -> AppResult<Response> {
    let p = provider(&st, &headers, id).await?;
    let request_id = format!("_{}", crypto::random_hex(20));
    let relay = crypto::random_token(24);
    sqlx::query("INSERT INTO saml_requests (id, provider_id, relay_state) VALUES ($1, $2, $3)").bind(&request_id).bind(id).bind(&relay).execute(&st.db).await?;
    let xml = authn_request(&request_id, Utc::now(), &p.sso_url, &p.acs_url, &p.sp_entity_id);
    let mut url = url::Url::parse(&p.sso_url).map_err(|e| AppError::Upstream(e.to_string()))?;
    url.query_pairs_mut().append_pair("SAMLRequest", &deflate_b64(&xml)).append_pair("RelayState", &relay);
    Ok(redirect(url.as_str()))
}

#[derive(Deserialize)]
struct AcsForm {
    #[serde(rename = "SAMLResponse")]
    saml_response: Option<String>,
    #[serde(rename = "RelayState")]
    relay_state: Option<String>,
}

async fn acs(State(st): State<St>, info: ReqInfo, headers: HeaderMap, Path(id): Path<Uuid>, Form(f): Form<AcsForm>) -> Response {
    let fail = |reason: String| {
        let (st, info) = (st.clone(), info.clone());
        async move {
            audit::security(&st, "auth.sso_failed", "warning", None, &info, json!({"provider": id, "protocol": "saml", "reason": reason})).await;
            redirect("/login?error=sso")
        }
    };
    let p = match provider(&st, &headers, id).await {
        Ok(p) => p,
        Err(_) => return fail("unknown or disabled provider".into()).await,
    };
    let Some(encoded) = f.saml_response.filter(|s| s.len() <= MAX_RESPONSE_BYTES * 2) else { return fail("missing or oversized SAMLResponse".into()).await };
    let Some(xml) = B64.decode(squeeze(&encoded)).ok().and_then(|b| String::from_utf8(b).ok()) else { return fail("SAMLResponse is not base64-encoded text".into()).await };
    let keys: Vec<RsaPublicKey> = match keys_from_certificates(&p.certificates) {
        Ok(k) => k.into_iter().map(|k| k.0).collect(),
        Err(e) => return fail(format!("stored certificate problem: {e}")).await,
    };
    let expect = Expect { idp_entity_id: &p.idp_entity_id, sp_entity_id: &p.sp_entity_id, acs_url: &p.acs_url, now: Utc::now(), keys: &keys };
    let assertion = match read_response(&xml, &expect) {
        Ok(a) => a,
        Err(e) => return fail(e).await,
    };
    // The response must answer a request this server made, recently, and only once.
    match &assertion.in_response_to {
        Some(request) => {
            let known = sqlx::query("DELETE FROM saml_requests WHERE id = $1 AND provider_id = $2 AND created_at > now() - interval '10 minutes' RETURNING relay_state").bind(request).bind(id).fetch_optional(&st.db).await;
            let Ok(Some(row)) = known else { return fail("the response answers an unknown, reused or expired request".into()).await };
            if f.relay_state.as_deref() != Some(row.get::<String, _>("relay_state").as_str()) {
                return fail("RelayState does not match the request".into()).await;
            }
        }
        None if p.allow_idp_initiated => {}
        None => return fail("sign-in was started at the identity provider, which this provider's settings do not allow".into()).await,
    }
    if assertion.id.is_empty() {
        return fail("the assertion has no ID".into()).await;
    }
    let fresh = sqlx::query("INSERT INTO saml_assertions (provider_id, assertion_id, expires_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING").bind(id).bind(&assertion.id).bind(assertion.expires + Duration::seconds(SKEW_SECONDS)).execute(&st.db).await;
    if !matches!(fresh, Ok(ref r) if r.rows_affected() == 1) {
        return fail("the assertion was already used (replay)".into()).await;
    }
    let who = identity(&assertion, &p.email_attribute, &p.name_attribute);
    let uid = match oidc::resolve_user(&st, &p.sso, &who).await {
        Ok(u) => u,
        Err(e) => return fail(e.public_message()).await,
    };
    match auth::start_session(&st, uid, &format!("saml:{}", p.sso.name), &info).await {
        Ok(cookie) => {
            audit::log_system(&st, Some(&info), who.email.as_deref().unwrap_or(&who.subject), "auth.login", "user", &uid.to_string(), "success", json!({"method": "saml", "provider": p.sso.name})).await;
            let mut r = redirect("/");
            r.headers_mut().insert(header::SET_COOKIE, cookie);
            r
        }
        Err(e) => fail(e.public_message()).await,
    }
}

/// Service provider metadata, to hand to the identity provider's administrator.
async fn metadata(State(st): State<St>, headers: HeaderMap, Path(id): Path<Uuid>) -> AppResult<Response> {
    let p = provider(&st, &headers, id).await?;
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<md:EntityDescriptor xmlns:md=\"{NS_MD}\" entityID=\"{}\"><md:SPSSODescriptor AuthnRequestsSigned=\"false\" WantAssertionsSigned=\"true\" protocolSupportEnumeration=\"{NS_SAMLP}\"><md:NameIDFormat>urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress</md:NameIDFormat><md:AssertionConsumerService Binding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\" Location=\"{}\" index=\"0\" isDefault=\"true\"/></md:SPSSODescriptor></md:EntityDescriptor>\n",
        xml_escape(&p.sp_entity_id),
        xml_escape(&p.acs_url)
    );
    Ok(([(header::CONTENT_TYPE, "application/samlmetadata+xml")], xml).into_response())
}

#[derive(Deserialize)]
struct MetadataBody {
    xml: Option<String>,
    url: Option<String>,
}

/// Read identity provider metadata (pasted, or fetched from an https address) so the
/// administrator does not have to copy fields by hand.
async fn import_metadata(State(st): State<St>, ctx: auth::Ctx, Json(b): Json<MetadataBody>) -> AppResult<Json<Value>> {
    crate::authz::require_global(&st, &ctx, "admin.security")?;
    let xml = match (b.xml, b.url) {
        (Some(x), _) if !x.trim().is_empty() => x,
        (_, Some(u)) => {
            // The same checks as any outbound request: https, no loopback or link-local
            // address (a cloud metadata service), no redirects to follow.
            let (client, parsed) = crate::integrations::guarded_client(&st, u.trim()).await.map_err(|e| bad(format!("the metadata address cannot be used: {e}")))?;
            let resp = client.get(parsed).send().await.and_then(|r| r.error_for_status()).map_err(|e| AppError::Upstream(format!("could not fetch the metadata: {}", e.without_url())))?;
            // Metadata is a small document. What is fetched is parsed, never echoed back.
            let mut resp = resp;
            let mut bytes: Vec<u8> = vec![];
            while let Some(chunk) = resp.chunk().await.map_err(|e| AppError::Upstream(e.without_url().to_string()))? {
                bytes.extend_from_slice(&chunk);
                if bytes.len() > MAX_METADATA {
                    return Err(bad("the metadata is too large"));
                }
            }
            String::from_utf8(bytes).map_err(|_| bad("the metadata is not UTF-8 text"))?
        }
        _ => return Err(bad("give the metadata XML or its address")),
    };
    Ok(Json(read_metadata(&xml).map_err(bad)?))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/auth/saml/{id}/start", "Auth", "Begin SAML single sign-on (redirects to the identity provider)", get(start))
        .public()
        .route("POST", "/api/v1/auth/saml/{id}/acs", "Auth", "SAML assertion consumer service: complete single sign-on", post(acs))
        .public()
        .route("GET", "/api/v1/auth/saml/{id}/metadata", "Auth", "SAML service provider metadata for this sign-in provider", get(metadata))
        .public()
        .route("POST", "/api/v1/admin/auth-providers/saml-metadata", "Admin: security", "Read identity provider metadata into provider settings", post(import_metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_with_a_great_many_attributes_is_refused_before_parsing() {
        let many = |n: usize, close: &str| format!("<a {}{close}", (0..n).map(|i| format!("a{i}=\"1\"")).collect::<Vec<_>>().join(" "));
        assert!(shape_within(&many(64, "/>"), MAX_DEPTH, MAX_ATTRIBUTES));
        assert!(!shape_within(&many(65, "/>"), MAX_DEPTH, MAX_ATTRIBUTES));
        assert!(!shape_within(&many(100_000, ""), MAX_DEPTH, MAX_ATTRIBUTES), "also when the tag never ends");
        assert!(!shape_within(&many(100_000, "/>").replace('"', "'"), MAX_DEPTH, MAX_ATTRIBUTES));
        // Quotes inside text, comments and CDATA are not attributes.
        let text = format!("<a>{}</a><!-- {} --><b><![CDATA[{}]]></b>", "\"x\" ".repeat(500), "'y' ".repeat(500), "\"z\" ".repeat(500));
        assert!(shape_within(&format!("<r>{text}</r>"), MAX_DEPTH, MAX_ATTRIBUTES));
        let started = std::time::Instant::now();
        assert!(parse(&many(100_000, "/>")).is_err());
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "{:?}", started.elapsed());
        // A real response is well within the limits.
        assert!(parse(include_str!("../tests/fixtures/saml/response-signed.xml")).is_ok());
    }

    fn canon(xml: &str, pick: &str) -> String {
        let doc = parse(xml).unwrap();
        let node = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == pick).unwrap();
        canonicalize(xml, node, None, &[])
    }

    #[test]
    fn canonical_form_orders_attributes_and_declares_only_used_namespaces() {
        let xml = r#"<a:root xmlns:a="urn:a" xmlns:b="urn:b" xmlns:unused="urn:u"><a:item  b:z="1"   y="2" a:x='3'>t &amp; &lt;x&gt; "q"</a:item></a:root>"#;
        assert_eq!(canon(xml, "item"), r#"<a:item xmlns:a="urn:a" xmlns:b="urn:b" y="2" a:x="3" b:z="1">t &amp; &lt;x&gt; "q"</a:item>"#);
        // A namespace is declared where it is first used and not repeated below.
        assert_eq!(canon(xml, "root"), r#"<a:root xmlns:a="urn:a"><a:item xmlns:b="urn:b" y="2" a:x="3" b:z="1">t &amp; &lt;x&gt; "q"</a:item></a:root>"#);
    }

    #[test]
    fn canonical_form_handles_default_namespaces_comments_and_empty_elements() {
        let xml = "<r xmlns=\"urn:d\"><e/><!-- gone --><n xmlns=\"\"><m/></n><e a=\"x&#10;y&#9;z\"></e></r>";
        assert_eq!(canon(xml, "r"), "<r xmlns=\"urn:d\"><e></e><n xmlns=\"\"><m></m></n><e a=\"x&#xA;y&#x9;z\"></e></r>");
        // Taken on its own, an element in no namespace needs no declaration at all.
        assert_eq!(canon(xml, "n"), "<n><m></m></n>");
    }

    #[test]
    fn inclusive_prefixes_are_declared_even_when_unused() {
        let xml = r#"<r xmlns:a="urn:a" xmlns:xs="urn:xs"><a:e>xs:string</a:e></r>"#;
        let doc = parse(xml).unwrap();
        let e = doc.descendants().find(|n| n.tag_name().name() == "e").unwrap();
        assert_eq!(canonicalize(xml, e, None, &["xs".to_string()]), r#"<a:e xmlns:a="urn:a" xmlns:xs="urn:xs">xs:string</a:e>"#);
    }

    #[test]
    fn text_split_by_a_comment_is_read_whole() {
        let xml = "<NameID>admin@example.org<!---->.attacker.test</NameID>";
        let doc = parse(xml).unwrap();
        assert_eq!(full_text(doc.root_element()), "admin@example.org.attacker.test");
    }

    #[test]
    fn documents_with_a_dtd_are_refused() {
        let bomb = "<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;\">]><r>&b;</r>";
        assert!(parse(bomb).is_err());
        let xxe = "<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><r>&x;</r>";
        assert!(parse(xxe).is_err());
    }

    #[test]
    fn deeply_nested_documents_are_refused_before_the_parser_sees_them() {
        let nest = |n: usize| format!("{}{}", "<a>".repeat(n), "</a>".repeat(n));
        assert!(parse(&nest(MAX_DEPTH)).is_ok());
        assert!(parse(&nest(MAX_DEPTH + 1)).unwrap_err().contains("nested too deeply"));
        // Deep enough to overflow the parser's stack if it were ever reached.
        assert!(parse(&nest(200_000)).unwrap_err().contains("nested too deeply"));
        assert!(parse(&"<a>".repeat(200_000)).is_err(), "unclosed");
        assert!(parse(&format!("{}{}", "<a b='x'>".repeat(100_000), "</a>".repeat(100_000))).is_err());
    }

    #[test]
    fn the_depth_scanner_is_not_fooled_by_what_merely_looks_like_tags() {
        let deep = "<a>".repeat(60);
        // Closing tags inside comments, CDATA, processing instructions and attribute values
        // do not close anything, so they must not hide real depth.
        for decoy in ["<!-- </a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a> -->", "<![CDATA[</a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a>]]>", "<?pi </a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a></a>?>"] {
            let xml = format!("<r>{}{decoy}{}", "<a>".repeat(30), "<a>".repeat(30));
            assert!(!nesting_within(&xml, MAX_DEPTH), "{decoy:.12}");
        }
        // A `/>` or `>` inside an attribute value does not end or self-close the tag.
        assert!(!nesting_within(&"<a x=\"/>\">".repeat(60), MAX_DEPTH));
        assert!(!nesting_within(&"<a x='>' y=\"/>\">".repeat(60), MAX_DEPTH));
        // Self-closing elements and siblings add no depth, however many there are.
        assert!(nesting_within(&format!("<r>{}</r>", "<a/><b x='1'/><c></c>".repeat(5_000)), MAX_DEPTH));
        assert!(nesting_within(&format!("<r>{}</r>", "<a><b><c>t &gt; u</c></b></a>".repeat(5_000)), MAX_DEPTH));
        assert!(!nesting_within(&deep, MAX_DEPTH));
        // Malformed tails are left to the parser, which fails before going deeper.
        for tail in ["<!-- never closed", "<![CDATA[ never closed", "<? never closed", "<a b=\"never closed", "<!DOCTYPE x>"] {
            let xml = format!("<r>{tail}{deep}");
            assert!(nesting_within(&xml, MAX_DEPTH), "{tail}");
            assert!(parse(&xml).is_err(), "{tail}");
        }
        // An end tag that never ends swallows the next tag; what follows is still counted.
        assert!(parse(&format!("<r></never-closed{deep}")).is_err());
    }

    #[test]
    fn requests_are_well_formed_and_escape_their_fields() {
        let xml = authn_request("_abc", DateTime::from_timestamp(1_770_000_000, 0).unwrap(), "https://idp.test/sso?a=1&b=2", "https://sp.test/acs", "https://sp.test/\"meta\"");
        let doc = parse(&xml).unwrap();
        let root = doc.root_element();
        assert!(is(root, NS_SAMLP, "AuthnRequest"));
        assert_eq!(root.attribute("Destination"), Some("https://idp.test/sso?a=1&b=2"));
        assert_eq!(root.attribute("IssueInstant"), Some("2026-02-02T02:40:00Z"));
        assert_eq!(full_text(child(root, NS_SAML, "Issuer").unwrap()), "https://sp.test/\"meta\"");
        // The redirect binding's encoding inflates back to the same XML.
        let raw = B64.decode(deflate_b64(&xml)).unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(&mut flate2::read::DeflateDecoder::new(raw.as_slice()), &mut out).unwrap();
        assert_eq!(out, xml);
    }

    #[test]
    fn metadata_is_read() {
        let xml = r#"<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="https://idp.test/entity">
          <IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
            <KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>
              QUJD
              REVG</ds:X509Certificate></ds:X509Data></ds:KeyInfo></KeyDescriptor>
            <KeyDescriptor use="encryption"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>WFla</ds:X509Certificate></ds:X509Data></ds:KeyInfo></KeyDescriptor>
            <SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="https://idp.test/post"/>
            <SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.test/redirect"/>
          </IDPSSODescriptor></EntityDescriptor>"#;
        let m = read_metadata(xml).unwrap();
        assert_eq!(m, json!({"issuer": "https://idp.test/entity", "sso_url": "https://idp.test/redirect", "certificates": ["QUJDREVG"]}));
        assert!(read_metadata("<EntityDescriptor xmlns=\"urn:oasis:names:tc:SAML:2.0:metadata\" entityID=\"x\"/>").is_err());
    }

    #[test]
    fn identity_prefers_configured_attributes_then_well_known_ones() {
        let mut a = Assertion { id: "_1".into(), name_id: "u-123".into(), name_id_format: String::new(), attributes: HashMap::new(), in_response_to: None, expires: Utc::now() };
        assert_eq!(identity(&a, "", "").email, None, "an opaque NameID is not an email address");
        a.attributes.insert("mail".into(), vec!["ada@example.org".into()]);
        a.attributes.insert("givenName".into(), vec!["Ada".into()]);
        a.attributes.insert("sn".into(), vec!["Lovelace".into()]);
        a.attributes.insert("work".into(), vec!["ada@work.example".into()]);
        let i = identity(&a, "", "");
        assert_eq!((i.email.as_deref(), i.name.as_deref(), i.subject.as_str()), (Some("ada@example.org"), Some("Ada Lovelace"), "u-123"));
        assert_eq!(identity(&a, "work", "").email.as_deref(), Some("ada@work.example"));
        assert_eq!(identity(&a, "missing", "").email, None, "a configured attribute that is absent is not silently replaced by another one");
        a.name_id = "ada@id.example".into();
        assert_eq!(identity(&a, "missing", "").email.as_deref(), Some("ada@id.example"));
    }
}
