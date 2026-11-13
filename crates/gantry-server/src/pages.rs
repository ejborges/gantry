//! What a person sees when something goes wrong: for each response code, what happened,
//! what they can do, and how to reach the people who run this installation.
//!
//! API clients always get problem documents (JSON). A browser that navigates straight to an
//! address that fails (a file link, a manual page, a sign-in callback) gets this page instead,
//! with the same status code. The web application shows the same texts on its own pages, read
//! from `GET /api/v1/about/status-codes`, so there is one place where they are written.

use crate::error::AppResult;
use crate::http::Api;
use crate::{settings, AppState, St};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde_json::{json, Value};

pub struct StatusText {
    pub code: u16,
    pub title: &'static str,
    /// What happened, in one or two sentences.
    pub meaning: &'static str,
    /// What the person can do about it.
    pub action: &'static str,
}

pub const STATUS_TEXTS: &[StatusText] = &[
    StatusText { code: 400, title: "The request could not be understood", meaning: "The address or the information sent with it was not in a form the server accepts.", action: "Go back and try again. If you followed a link, it may have been cut short when it was copied." },
    StatusText { code: 401, title: "You need to sign in", meaning: "This needs a signed-in account, and you are not signed in or your session has ended.", action: "Sign in, then open the link again." },
    StatusText { code: 403, title: "You do not have access to this", meaning: "You are signed in, but your account is not allowed to see or do this.", action: "If you think you should have access, ask the owner of the project or space, or an administrator, to add you." },
    StatusText { code: 404, title: "This does not exist, or you cannot see it", meaning: "Nothing is at this address for your account. It may have been moved, archived or deleted, or it may be in a project or space you are not a member of.", action: "Check the address, use search, or ask the person who sent you the link." },
    StatusText { code: 405, title: "That is not possible at this address", meaning: "The address exists but does not support what was asked of it.", action: "Go back and use the buttons and links in the application." },
    StatusText { code: 408, title: "The request took too long to arrive", meaning: "The server stopped waiting because the request did not arrive in time, usually because of a slow or interrupted connection.", action: "Check your connection and try again." },
    StatusText { code: 409, title: "This changed while you were working on it", meaning: "Someone else changed the same thing first, or the change conflicts with something that already exists.", action: "Reload to see the current state, then make your change again." },
    StatusText { code: 410, title: "This is no longer available", meaning: "It existed once and has been removed for good.", action: "Ask the person who sent you the link whether there is a replacement." },
    StatusText { code: 413, title: "That is too large", meaning: "The file or the text you sent is larger than this installation accepts.", action: "Send a smaller file, or split it. An administrator can tell you the limit." },
    StatusText { code: 415, title: "That kind of content is not accepted here", meaning: "The server does not accept this type of file or data at this address.", action: "Check the file type and try again." },
    StatusText { code: 422, title: "Something in the request is not valid", meaning: "The request was understood, but a value in it is missing or not allowed.", action: "Go back, correct the highlighted fields, and try again." },
    StatusText { code: 429, title: "Too many requests", meaning: "Too much was asked of the server in a short time from your account or your network, so it is briefly refusing more.", action: "Wait a minute, then try again." },
    StatusText { code: 500, title: "Something went wrong on the server", meaning: "The server ran into a problem it did not expect. It was recorded. Nothing you did caused it.", action: "Try again in a moment. If it keeps happening, tell support and include the reference below." },
    StatusText { code: 501, title: "This is not set up", meaning: "This installation does not have the feature this address needs.", action: "Ask an administrator whether it can be turned on." },
    StatusText { code: 502, title: "The application is not answering", meaning: "The server in front of the application could not reach it. It may be restarting or being upgraded.", action: "Wait a minute and reload. If it lasts, tell support." },
    StatusText { code: 503, title: "Temporarily unavailable", meaning: "The application is running but cannot serve requests right now, for example during maintenance or because the database is unreachable.", action: "Wait a few minutes and reload. If it lasts, tell support." },
    StatusText { code: 504, title: "The application took too long to answer", meaning: "The server in front of the application gave up waiting for an answer.", action: "Try again. If you were asking for something large, narrow it down." },
];

/// The text for a code; unknown codes get the general text for their class.
pub fn text_for(code: u16) -> StatusText {
    if let Some(t) = STATUS_TEXTS.iter().find(|t| t.code == code) {
        return StatusText { code, title: t.title, meaning: t.meaning, action: t.action };
    }
    if code >= 500 {
        StatusText { code, title: "The server could not complete the request", meaning: "Something went wrong on the server's side.", action: "Try again in a moment. If it keeps happening, tell support and include the reference below." }
    } else {
        StatusText { code, title: "The request could not be completed", meaning: "The server refused or could not process the request.", action: "Go back and try again." }
    }
}

pub fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// A link is shown only when it is plainly a web or mail address.
fn safe_url(url: &str) -> Option<&str> {
    let u = url.trim();
    (u.starts_with("https://") || u.starts_with("http://")).then_some(u).filter(|u| !u.chars().any(|c| c.is_control() || c == '"' || c == '<' || c == '>' || c.is_whitespace()))
}

fn safe_email(email: &str) -> Option<&str> {
    let e = email.trim();
    (e.len() <= 254 && e.split('@').count() == 2 && !e.chars().any(|c| c.is_control() || c.is_whitespace() || "\"<>?&,;:".contains(c))).then_some(e).filter(|e| !e.starts_with('@') && !e.ends_with('@'))
}

/// How to reach support, as HTML. Empty when the administrator has not said how.
fn support_html(support: &Value) -> String {
    let field = |k: &str| support[k].as_str().unwrap_or_default().trim().to_string();
    let (name, email, url, text) = (field("name"), field("email"), field("url"), field("text"));
    let mut lines = vec![];
    if !text.is_empty() {
        lines.push(format!("<p>{}</p>", escape(&text)));
    }
    if let Some(e) = safe_email(&email) {
        lines.push(format!("<p>Email: <a href=\"mailto:{0}\">{0}</a></p>", escape(e)));
    }
    if let Some(u) = safe_url(&url) {
        lines.push(format!("<p>Web: <a href=\"{0}\" rel=\"noopener noreferrer\">{0}</a></p>", escape(u)));
    }
    if lines.is_empty() {
        return "<p>Contact the people who run this installation in your organization.</p>".into();
    }
    if !name.is_empty() {
        lines.insert(0, format!("<p><strong>{}</strong></p>", escape(&name)));
    }
    lines.join("")
}

const STYLE: &str = "
:root{color-scheme:light;--bg:#f3f5f8;--surface:#fff;--ink:#16202b;--muted:#5a6877;--line:#d9dfe7;--action:#1a56c4}
:root[data-theme=dark]{color-scheme:dark;--bg:#10171f;--surface:#18212c;--ink:#e7ecf2;--muted:#99a6b4;--line:#2a3643;--action:#73a5ff}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px 16px;background:var(--bg);color:var(--ink);font:16px/1.5 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif}
main{width:100%;max-width:560px;padding:28px 28px 22px;background:var(--surface);border:1px solid var(--line);border-radius:11px}
.code{margin:0 0 4px;color:var(--muted);font-size:14px;font-weight:600;letter-spacing:.04em}
h1{margin:0 0 12px;font-size:24px;line-height:1.25}
h2{margin:22px 0 6px;font-size:15px}
p{margin:0 0 10px}
a{color:var(--action)}
a:focus-visible{outline:2px solid var(--action);outline-offset:2px;border-radius:3px}
.actions{display:flex;flex-wrap:wrap;gap:10px;margin-top:18px}
.btn{display:inline-block;padding:8px 14px;border:1px solid var(--line);border-radius:7px;color:var(--ink);text-decoration:none;font-weight:500}
.btn.primary{background:var(--action);border-color:var(--action);color:var(--surface)}
.ref{margin-top:18px;padding-top:12px;border-top:1px solid var(--line);color:var(--muted);font-size:13px}
code{font-family:ui-monospace,Menlo,Consolas,monospace}
";

/// The page for a response code. It needs no script to be read; the theme script only picks
/// light or dark, and the page is light without it.
pub fn render(site_name: &str, support: &Value, code: u16, request_id: &str, signed_in: bool) -> String {
    let t = text_for(code);
    let actions = if code == 401 {
        "<a class=\"btn primary\" href=\"/login\">Sign in</a>".to_string()
    } else {
        format!("<a class=\"btn primary\" href=\"/\">{}</a>", if signed_in { "Go to my work" } else { "Go to the start page" })
    };
    let reference = if request_id.is_empty() { String::new() } else { format!(" Reference: <code>{}</code>.", escape(request_id)) };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"robots\" content=\"noindex\">\n<title>{title} · {site}</title>\n<link rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\">\n<script src=\"/theme.js\"></script>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n<p class=\"code\">Error {code}</p>\n<h1>{title}</h1>\n<p>{meaning}</p>\n<h2>What you can do</h2>\n<p>{action}</p>\n<div class=\"actions\">{actions}</div>\n<h2>Getting help</h2>\n{support}\n<p class=\"ref\">When you ask for help, say what you were doing and when.{reference}</p>\n</main>\n</body>\n</html>\n",
        title = escape(t.title),
        site = escape(site_name),
        code = code,
        meaning = escape(t.meaning),
        action = escape(t.action),
        actions = actions,
        support = support_html(support),
        reference = reference,
    )
}

/// True when the request is a person opening an address in a browser, not a program or the
/// web application fetching data.
pub fn is_navigation(method: &Method, headers: &HeaderMap) -> bool {
    if method != Method::GET {
        return false;
    }
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default();
    match get("sec-fetch-mode") {
        "navigate" => get("sec-fetch-dest") != "iframe" || get("accept").contains("text/html"),
        "" => get("accept").trim_start().starts_with("text/html"),
        _ => false,
    }
}

/// Replace a failed response with the page for its code.
pub async fn page(st: &AppState, status: StatusCode, request_id: &str, signed_in: bool) -> Response {
    let site = settings::get(st, "site").await;
    let html = render(site["name"].as_str().unwrap_or("Gantry"), &site["support"], status.as_u16(), request_id, signed_in);
    let mut resp = (status, html).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// Public: the same texts, for the web application's own pages.
async fn status_codes(State(st): State<St>) -> AppResult<Json<Value>> {
    let site = settings::get(&st, "site").await;
    Ok(Json(json!({
        "codes": STATUS_TEXTS.iter().map(|t| json!({"code": t.code, "title": t.title, "meaning": t.meaning, "action": t.action})).collect::<Vec<_>>(),
        "support": site["support"],
    })))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/about/status-codes", "About", "What each response code means for a person, what they can do, and how to reach support", get(status_codes)).public()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_code_has_complete_text_and_unknown_codes_fall_back() {
        for t in STATUS_TEXTS {
            assert!((400..600).contains(&t.code));
            assert!(!t.title.is_empty() && t.meaning.ends_with('.') && t.action.ends_with('.'), "{}", t.code);
        }
        for must in [400, 401, 403, 404, 409, 413, 422, 429, 500, 502, 503, 504] {
            assert!(STATUS_TEXTS.iter().any(|t| t.code == must), "{must}");
        }
        assert_eq!(text_for(418).title, "The request could not be completed");
        assert_eq!(text_for(599).title, "The server could not complete the request");
    }

    #[test]
    fn the_page_escapes_what_administrators_and_requests_supply() {
        let support = json!({"name": "<b>Help</b>", "email": "help@example.org", "url": "https://help.example.org/?a=1&b=2", "text": "Call <script>alert(1)</script> 555"});
        let html = render("Acme <Ops>", &support, 404, "abc\"><script>", true);
        assert!(!html.contains("<script>alert"), "support text is escaped");
        assert!(html.contains("Acme &lt;Ops&gt;") && html.contains("&lt;b&gt;Help&lt;/b&gt;"));
        assert!(html.contains("mailto:help@example.org") && html.contains("https://help.example.org/?a=1&amp;b=2"));
        assert!(html.contains("abc&quot;&gt;&lt;script&gt;"));
        assert!(html.contains("Error 404") && html.contains("Go to my work") && !html.contains("javascript:"));
    }

    #[test]
    fn addresses_that_are_not_plain_web_or_mail_addresses_are_not_linked() {
        for bad in ["javascript:alert(1)", "data:text/html,x", "//evil.example", "https://a.example/\" onclick=\"x", "ftp://a.example"] {
            let html = render("Gantry", &json!({"url": bad, "text": "t"}), 500, "", false);
            assert!(!html.contains("href=\"javascript") && !html.contains("onclick") && !html.contains("href=\"data:") && !html.contains("href=\"//") && !html.contains("ftp://"), "{bad}");
        }
        for bad in ["a@b@c", "x y@example.org", "\"><a>@example.org", "@example.org", "name@"] {
            assert!(safe_email(bad).is_none(), "{bad}");
        }
        let none = render("Gantry", &json!({"name": "", "email": "", "url": "", "text": ""}), 403, "", false);
        assert!(none.contains("Contact the people who run this installation"));
        assert!(render("Gantry", &json!({}), 401, "", false).contains("href=\"/login\""));
    }

    #[test]
    fn only_a_browser_opening_an_address_counts_as_navigation() {
        let h = |pairs: &[(&str, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
            }
            m
        };
        assert!(is_navigation(&Method::GET, &h(&[("sec-fetch-mode", "navigate"), ("sec-fetch-dest", "document"), ("accept", "text/html,application/xhtml+xml")])));
        assert!(is_navigation(&Method::GET, &h(&[("accept", "text/html")])), "older browsers send no fetch metadata");
        assert!(!is_navigation(&Method::GET, &h(&[("sec-fetch-mode", "cors"), ("accept", "application/json")])));
        assert!(!is_navigation(&Method::GET, &h(&[("accept", "application/json")])));
        assert!(!is_navigation(&Method::GET, &h(&[("accept", "*/*")])), "curl and scripts");
        assert!(!is_navigation(&Method::POST, &h(&[("sec-fetch-mode", "navigate"), ("accept", "text/html")])));
    }
}
