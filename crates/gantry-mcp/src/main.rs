//! `gantry-mcp`: a stdio bridge for MCP clients that launch servers as local processes.
//!
//! It forwards each JSON-RPC message from standard input to a Gantry server's `/mcp` endpoint and
//! writes the reply to standard output. Configuration is by environment:
//!
//! * `GANTRY_URL`   — base URL of the server, e.g. `https://gantry.example.org`
//! * `GANTRY_TOKEN` — an API token (`gty_…`)
//!
//! Nothing but protocol messages is written to standard output; diagnostics go to standard error.

use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Headers the 2026-07-28 transport requires, derived from the message body.
fn modern_headers(msg: &Value) -> Vec<(&'static str, String)> {
    let Some(version) = msg["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str() else { return vec![] };
    let method = msg["method"].as_str().unwrap_or_default();
    let mut h = vec![("MCP-Protocol-Version", version.to_string()), ("Mcp-Method", method.to_string())];
    let name = match method {
        "tools/call" | "prompts/get" => msg["params"]["name"].as_str(),
        "resources/read" => msg["params"]["uri"].as_str(),
        _ => None,
    };
    if let Some(n) = name {
        let safe = n.bytes().all(|b| (0x21..0x7f).contains(&b)) && !n.starts_with("=?base64?");
        h.push(("Mcp-Name", if safe { n.to_string() } else { format!("=?base64?{}?=", base64::engine::general_purpose::STANDARD.encode(n)) }));
    }
    h
}

fn error_reply(id: &Value, message: String) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": message}})
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (Ok(base), Ok(token)) = (std::env::var("GANTRY_URL"), std::env::var("GANTRY_TOKEN")) else {
        eprintln!("gantry-mcp: set GANTRY_URL (for example https://gantry.example.org) and GANTRY_TOKEN (an API token)");
        std::process::exit(2);
    };
    let endpoint = format!("{}/mcp", base.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).user_agent(concat!("gantry-mcp/", env!("CARGO_PKG_VERSION"))).build().expect("HTTP client");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("gantry-mcp: ignoring a line that is not JSON: {e}");
                continue;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let mut req = client.post(&endpoint).bearer_auth(&token).header("Accept", "application/json, text/event-stream").json(&msg);
        for (k, v) in modern_headers(&msg) {
            req = req.header(k, v);
        }
        let reply = match req.send().await {
            Ok(resp) if resp.status() == reqwest::StatusCode::ACCEPTED => None,
            Ok(resp) => {
                let status = resp.status();
                match resp.json::<Value>().await {
                    Ok(v) if v.get("jsonrpc").is_some() => Some(v),
                    _ => Some(error_reply(&id, format!("the Gantry server answered HTTP {status}"))),
                }
            }
            Err(e) => Some(error_reply(&id, format!("cannot reach the Gantry server: {}", e.without_url()))),
        };
        // Notifications (no id) never get a reply.
        if let (Some(r), false) = (reply, id.is_null()) {
            let mut text = r.to_string();
            text.push('\n');
            if out.write_all(text.as_bytes()).await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_messages_get_no_extra_headers() {
        assert!(modern_headers(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).is_empty());
    }

    #[test]
    fn modern_messages_mirror_method_and_name() {
        let m = json!({"method": "tools/call", "params": {"name": "search", "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}});
        assert_eq!(modern_headers(&m), vec![("MCP-Protocol-Version", "2026-07-28".to_string()), ("Mcp-Method", "tools/call".to_string()), ("Mcp-Name", "search".to_string())]);
        let r = json!({"method": "resources/read", "params": {"uri": "gantry://page/é", "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}});
        assert_eq!(modern_headers(&r)[2].1, "=?base64?Z2FudHJ5Oi8vcGFnZS/DqQ==?=");
    }
}
