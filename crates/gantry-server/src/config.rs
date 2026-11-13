//! Configuration: one TOML file plus a small set of environment overrides.
//!
//! Every key has a safe default except `security.secret_key`, which must be set
//! in production (`gantry config generate-key` prints one).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    pub database: Database,
    pub auth: Auth,
    pub security: Security,
    pub logging: Logging,
    pub storage: Storage,
    pub ai: Ai,
    pub worker: Worker,
    pub addons: Addons,
    pub integrations: Integrations,
    pub email: Email,
    pub realtime: Realtime,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    /// Address the HTTP listener binds to. Keep it on loopback behind nginx.
    pub bind: String,
    /// External base URL, e.g. `https://gantry.example.org`. Drives cookie security and links.
    pub public_url: String,
    /// Directory holding the built web application.
    pub web_root: PathBuf,
    /// Directory holding the built web manual.
    pub manual_root: PathBuf,
    /// Peers allowed to supply `X-Real-IP` / `X-Forwarded-For`.
    pub trusted_proxies: Vec<String>,
    /// Maximum request body in megabytes (attachments included).
    pub max_body_mb: usize,
    /// Requests per minute allowed per user (or per address when anonymous).
    pub rate_limit_per_minute: u32,
    /// Bearer token for `/metrics`. Empty means loopback clients only.
    pub metrics_token: String,
}
impl Default for Server {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8600".into(),
            public_url: String::new(),
            web_root: "/usr/share/gantry/web".into(),
            manual_root: "/usr/share/gantry/manual".into(),
            trusted_proxies: vec!["127.0.0.1".into(), "::1".into()],
            max_body_mb: 25,
            rate_limit_per_minute: 600,
            metrics_token: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Database {
    /// A password in the address is replaced by `***` whenever the configuration is printed.
    #[serde(serialize_with = "redact_url_password")]
    pub url: String,
    pub max_connections: u32,
}
impl Default for Database {
    fn default() -> Self {
        Self { url: "postgres://gantry@%2Fvar%2Frun%2Fpostgresql/gantry".into(), max_connections: 20 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Auth {
    pub allow_local_login: bool,
    pub password_min_length: usize,
    pub session_idle_minutes: i64,
    pub session_absolute_hours: i64,
    pub lockout_threshold: i32,
    pub lockout_minutes: i64,
    pub login_attempts_per_ip: i64,
}
impl Default for Auth {
    fn default() -> Self {
        Self {
            allow_local_login: true,
            password_min_length: 12,
            session_idle_minutes: 480,
            session_absolute_hours: 168,
            lockout_threshold: 10,
            lockout_minutes: 15,
            login_attempts_per_ip: 30,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Security {
    /// Base64 of 32 random bytes. Encrypts stored secrets (OIDC client secrets, integration tokens).
    #[serde(skip_serializing)]
    pub secret_key: String,
    /// File holding the key instead of `secret_key`, so the key can live outside the
    /// configuration file with tighter permissions. Packages use /etc/gantry/secret.key.
    pub secret_key_file: PathBuf,
    /// Refuse outbound integration calls to private (RFC 1918 / ULA) addresses.
    pub block_private_networks: bool,
    /// Permit plain-http outbound integration URLs (lab use only).
    pub allow_insecure_outbound: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    /// `json` (production, journald-friendly) or `text` (development).
    pub format: String,
    /// `error`, `warn`, `info`, `debug` or `trace`; also accepts `tracing` filter directives.
    pub level: String,
}
impl Default for Logging {
    fn default() -> Self {
        Self { format: "json".into(), level: "info".into() }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Storage {
    pub blob: Blob,
    pub search: Search,
    pub vector: Vector,
    pub kv: Kv,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Blob {
    /// `fs` or `s3`.
    pub backend: String,
    pub path: PathBuf,
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    #[serde(skip_serializing)]
    pub secret_key: String,
}
impl Default for Blob {
    fn default() -> Self {
        Self {
            backend: "fs".into(),
            path: "/var/lib/gantry/blobs".into(),
            endpoint: String::new(),
            bucket: "gantry".into(),
            region: "us-east-1".into(),
            access_key: String::new(),
            secret_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Search {
    /// `pg` or `opensearch`.
    pub backend: String,
    pub url: String,
    pub index: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
}
impl Default for Search {
    fn default() -> Self {
        Self { backend: "pg".into(), url: String::new(), index: "gantry".into(), username: String::new(), password: String::new() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vector {
    /// `pg_array` (portable, exact) or `pgvector` (HNSW index; needs the pgvector extension).
    pub backend: String,
}
impl Default for Vector {
    fn default() -> Self {
        Self { backend: "pg_array".into() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Kv {
    /// `pg` or `valkey`.
    pub backend: String,
    /// `valkey://[:password@]host:port[/db]` (the Redis protocol; `redis://` is accepted).
    #[serde(skip_serializing)]
    pub url: String,
}
impl Default for Kv {
    fn default() -> Self {
        Self { backend: "pg".into(), url: String::new() }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ai {
    pub embeddings: Embeddings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Embeddings {
    /// `none`, `hash` (built-in, offline, lexical quality) or `openai_compatible`
    /// (any server exposing `POST {base_url}/embeddings`: NVIDIA NIM, vLLM, Ollama, TEI).
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub dimensions: usize,
    /// Semantic matches scoring below this cosine similarity are dropped, so a query about
    /// nothing in particular returns nothing instead of its nearest unrelated neighbors.
    pub min_similarity: f64,
}
impl Default for Embeddings {
    fn default() -> Self {
        Self { provider: "none".into(), base_url: String::new(), model: String::new(), api_key: String::new(), dimensions: 256, min_similarity: 0.2 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Worker {
    /// Run the job worker inside the server process. Turn off when dedicated worker nodes exist.
    pub embedded: bool,
    pub concurrency: usize,
    pub poll_seconds: u64,
}
impl Default for Worker {
    fn default() -> Self {
        Self { embedded: true, concurrency: 4, poll_seconds: 2 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Addons {
    /// Directory scanned for add-on packs (`<dir>/<id>/pack.json`).
    pub dir: PathBuf,
}
impl Default for Addons {
    fn default() -> Self {
        Self { dir: "/usr/share/gantry/addons".into() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Integrations {
    pub timeout_seconds: u64,
}
impl Default for Integrations {
    fn default() -> Self {
        Self { timeout_seconds: 15 }
    }
}

/// Outgoing mail. Gantry hands messages to an SMTP server; it does not receive mail.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Email {
    /// Send notification email. Also needs `host` and `from`, and `server.public_url` for links.
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    /// `starttls` (usual on port 587), `tls` (implicit TLS, usual on port 465) or `none`
    /// (a relay on the same machine or a trusted network only).
    pub encryption: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    /// Read the password from this file instead (not readable by other users).
    pub password_file: PathBuf,
    /// Sender, e.g. `Gantry <gantry@example.org>`.
    pub from: String,
    /// Optional address replies should go to, e.g. the service desk mailbox.
    pub reply_to: String,
    pub timeout_seconds: u64,
}
impl Default for Email {
    fn default() -> Self {
        Self { enabled: false, host: String::new(), port: 587, encryption: "starttls".into(), username: String::new(), password: String::new(), password_file: PathBuf::new(), from: String::new(), reply_to: String::new(), timeout_seconds: 20 }
    }
}
impl Email {
    pub fn enabled(&self) -> bool {
        self.enabled && !self.host.is_empty() && !self.from.is_empty()
    }
}

/// Live updates and concurrent editing over WebSocket.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Realtime {
    pub enabled: bool,
    /// Connections one account may hold at once (tabs and devices).
    pub max_connections_per_user: usize,
    /// Messages a connection may send per second before it is closed.
    pub max_messages_per_second: u32,
}
impl Default for Realtime {
    fn default() -> Self {
        Self { enabled: true, max_connections_per_user: 12, max_messages_per_second: 60 }
    }
}

impl Config {
    /// Load configuration from `path`, or from `GANTRY_CONFIG`, or from `/etc/gantry/gantry.toml`
    /// when it exists. Environment variables then override the handful of deployment-specific keys.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Config> {
        let candidate = path
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("GANTRY_CONFIG").map(PathBuf::from))
            .or_else(|| {
                let p = PathBuf::from("/etc/gantry/gantry.toml");
                p.exists().then_some(p)
            });
        let mut cfg = match candidate {
            Some(p) => Self::read_with_dropins(&p)?,
            None => Config::default(),
        };
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = env("GANTRY_DATABASE_URL") {
            cfg.database.url = v;
        }
        if let Some(v) = env("GANTRY_SECRET_KEY") {
            cfg.security.secret_key = v;
        }
        if cfg.security.secret_key.is_empty() && !cfg.security.secret_key_file.as_os_str().is_empty() {
            cfg.security.secret_key = read_secret_file(&cfg.security.secret_key_file)?;
        }
        if cfg.email.password.is_empty() && !cfg.email.password_file.as_os_str().is_empty() {
            cfg.email.password = read_secret_file(&cfg.email.password_file)?;
        }
        if let Some(v) = env("GANTRY_BIND") {
            cfg.server.bind = v;
        }
        if let Some(v) = env("GANTRY_PUBLIC_URL") {
            cfg.server.public_url = v;
        }
        if let Some(v) = env("GANTRY_WEB_ROOT") {
            cfg.server.web_root = v.into();
        }
        if let Some(v) = env("GANTRY_MANUAL_ROOT") {
            cfg.server.manual_root = v.into();
        }
        if let Some(v) = env("GANTRY_ADDONS_DIR") {
            cfg.addons.dir = v.into();
        }
        if let Some(v) = env("GANTRY_BLOB_PATH") {
            cfg.storage.blob.path = v.into();
        }
        if let Some(v) = env("GANTRY_LOG_FORMAT") {
            cfg.logging.format = v;
        }
        if let Some(v) = env("GANTRY_LOG_LEVEL") {
            cfg.logging.level = v;
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read `path`, then lay every `conf.d/*.toml` beside it on top in name order. Optional
    /// packages drop a small file there to switch a storage backend, so installing or removing
    /// one never edits the administrator's own file.
    fn read_with_dropins(path: &Path) -> anyhow::Result<Config> {
        let parse = |p: &Path| -> anyhow::Result<toml::Table> {
            let text = std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", p.display()))?;
            // Checked on its own first so that a mistake is reported against the file it is in.
            toml::from_str::<Config>(&text).map_err(|e| anyhow::anyhow!("invalid configuration in {}: {e}", p.display()))?;
            Ok(toml::from_str::<toml::Table>(&text)?)
        };
        let mut merged = parse(path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).join("conf.d");
        let mut dropins: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect()).unwrap_or_default();
        dropins.sort();
        for p in dropins {
            merge_tables(&mut merged, parse(&p)?);
        }
        toml::Value::Table(merged).try_into::<Config>().map_err(|e| anyhow::anyhow!("invalid configuration in {} with {}: {e}", path.display(), dir.display()))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let one_of = |name: &str, v: &str, allowed: &[&str]| -> anyhow::Result<()> {
            anyhow::ensure!(allowed.contains(&v), "{name} must be one of {allowed:?}, got {v:?}");
            Ok(())
        };
        one_of("logging.format", &self.logging.format, &["json", "text"])?;
        one_of("storage.blob.backend", &self.storage.blob.backend, &["fs", "s3"])?;
        one_of("storage.search.backend", &self.storage.search.backend, &["pg", "opensearch"])?;
        one_of("storage.vector.backend", &self.storage.vector.backend, &["pg_array", "pgvector"])?;
        one_of("storage.kv.backend", &self.storage.kv.backend, &["pg", "valkey"])?;
        one_of("ai.embeddings.provider", &self.ai.embeddings.provider, &["none", "hash", "openai_compatible"])?;
        one_of("email.encryption", &self.email.encryption, &["starttls", "tls", "none"])?;
        if self.email.enabled {
            anyhow::ensure!(!self.email.host.is_empty(), "email.host must be set when email.enabled is true");
            anyhow::ensure!(self.email.from.parse::<lettre::message::Mailbox>().is_ok(), "email.from must be a sender such as \"Gantry <gantry@example.org>\"");
            anyhow::ensure!(self.email.reply_to.is_empty() || self.email.reply_to.parse::<lettre::message::Mailbox>().is_ok(), "email.reply_to is not a valid address");
            anyhow::ensure!(!self.server.public_url.is_empty(), "server.public_url must be set when email.enabled is true: messages link back to the application");
        }
        anyhow::ensure!(self.realtime.max_connections_per_user >= 1 && self.realtime.max_messages_per_second >= 5, "realtime limits are too low to be usable");
        for entry in &self.server.trusted_proxies {
            // A mistyped entry would silently leave every client looking like the proxy.
            anyhow::ensure!(crate::http::net_contains(entry, std::net::IpAddr::from([127, 0, 0, 1])).is_some(), "server.trusted_proxies: {entry:?} is not an IP address or a network such as 10.0.0.0/8");
        }
        anyhow::ensure!(self.auth.password_min_length >= 8, "auth.password_min_length must be at least 8");
        anyhow::ensure!((16..=4096).contains(&self.ai.embeddings.dimensions), "ai.embeddings.dimensions must be between 16 and 4096");
        if !self.server.public_url.is_empty() {
            let u = url::Url::parse(&self.server.public_url).map_err(|e| anyhow::anyhow!("server.public_url: {e}"))?;
            anyhow::ensure!(matches!(u.scheme(), "http" | "https"), "server.public_url must be http or https");
        }
        Ok(())
    }

    pub fn secure_cookies(&self) -> bool {
        self.server.public_url.starts_with("https://")
    }

    /// Origin (`scheme://host[:port]`) of the public URL, if configured.
    pub fn public_origin(&self) -> Option<String> {
        url::Url::parse(&self.server.public_url).ok().map(|u| u.origin().ascii_serialization())
    }
}

fn redact_url_password<S: serde::Serializer>(url: &str, s: S) -> Result<S::Ok, S::Error> {
    match url::Url::parse(url) {
        Ok(mut u) if u.password().is_some() => {
            let _ = u.set_password(Some("***"));
            s.serialize_str(u.as_str())
        }
        _ => s.serialize_str(url),
    }
}

fn merge_tables(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge_tables(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

fn read_secret_file(path: &Path) -> anyhow::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).map_err(|e| anyhow::anyhow!("cannot read the secret file {}: {e}", path.display()))?.permissions().mode();
        anyhow::ensure!(mode & 0o007 == 0, "{} is readable by every user on this machine (mode {:o}); run: chmod 640 {}", path.display(), mode & 0o777, path.display());
    }
    let key = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("cannot read the secret file {}: {e}", path.display()))?;
    Ok(key.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn email_needs_a_host_a_sender_and_a_public_url() {
        let mut c = Config::default();
        c.email.enabled = true;
        assert!(c.validate().unwrap_err().to_string().contains("email.host"));
        c.email.host = "smtp.example.org".into();
        assert!(c.validate().unwrap_err().to_string().contains("email.from"));
        c.email.from = "Gantry <gantry@example.org>".into();
        assert!(c.validate().unwrap_err().to_string().contains("server.public_url"));
        c.server.public_url = "https://gantry.example.org".into();
        c.validate().unwrap();
        assert!(c.email.enabled());
        c.email.encryption = "ssl3".into();
        assert!(c.validate().is_err());
        assert!(!Config::default().email.enabled(), "email is off until configured");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = toml::from_str::<Config>("[server]\nbnd = \"x\"\n").unwrap_err().to_string();
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn bad_backend_is_rejected() {
        let mut c = Config::default();
        c.storage.blob.backend = "ftp".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn secrets_are_never_serialized() {
        let mut c = Config::default();
        c.security.secret_key = "TOPSECRET".into();
        c.storage.blob.secret_key = "TOPSECRET".into();
        c.storage.search.password = "TOPSECRET".into();
        c.storage.kv.url = "valkey://:TOPSECRET@h:6379".into();
        c.ai.embeddings.api_key = "TOPSECRET".into();
        c.email.password = "TOPSECRET".into();
        assert!(!serde_json::to_string(&c).unwrap().contains("TOPSECRET"));
    }

    #[test]
    fn https_public_url_enables_secure_cookies() {
        let mut c = Config::default();
        assert!(!c.secure_cookies());
        c.server.public_url = "https://gantry.example.org/".into();
        assert!(c.secure_cookies());
        assert_eq!(c.public_origin().as_deref(), Some("https://gantry.example.org"));
    }

    #[test]
    fn a_mistyped_trusted_proxy_is_rejected() {
        let mut c = Config::default();
        c.server.trusted_proxies = vec!["10.0.0.0/8".into(), "::1".into(), "192.168.1.10".into()];
        assert!(c.validate().is_ok());
        c.server.trusted_proxies = vec!["loadbalancer".into()];
        assert!(c.validate().unwrap_err().to_string().contains("trusted_proxies"));
    }

    #[test]
    fn a_database_password_is_not_printed() {
        let mut c = Config::default();
        c.database.url = "postgres://gantry:hunter2secret@db.internal:5432/gantry?sslmode=verify-full".into();
        let shown = toml::to_string(&c).unwrap();
        assert!(!shown.contains("hunter2secret"), "{shown}");
        assert!(shown.contains("postgres://gantry:***@db.internal:5432/gantry?sslmode=verify-full"), "{shown}");
        // An address without a password is shown unchanged.
        let shown = toml::to_string(&Config::default()).unwrap();
        assert!(shown.contains("postgres://gantry@%2Fvar%2Frun%2Fpostgresql/gantry"), "{shown}");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gantry-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("conf.d")).unwrap();
        d
    }

    #[test]
    fn drop_ins_override_the_main_file_in_name_order() {
        let d = scratch("dropins");
        std::fs::write(d.join("gantry.toml"), "[server]\nbind = \"127.0.0.1:9000\"\nmax_body_mb = 10\n[storage.kv]\nbackend = \"pg\"\n").unwrap();
        std::fs::write(d.join("conf.d/50-valkey.toml"), "[storage.kv]\nbackend = \"valkey\"\nurl = \"valkey://127.0.0.1:6379\"\n").unwrap();
        std::fs::write(d.join("conf.d/60-site.toml"), "[server]\nmax_body_mb = 64\n").unwrap();
        std::fs::write(d.join("conf.d/README"), "not configuration").unwrap();
        let c = Config::load(Some(&d.join("gantry.toml"))).unwrap();
        assert_eq!(c.server.bind, "127.0.0.1:9000", "keys a drop-in does not mention are kept");
        assert_eq!(c.server.max_body_mb, 64);
        assert_eq!(c.storage.kv.backend, "valkey");
        assert_eq!(c.storage.kv.url, "valkey://127.0.0.1:6379");
        assert_eq!(c.storage.search.backend, "pg");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn a_mistake_in_a_drop_in_names_that_file() {
        let d = scratch("badfile");
        std::fs::write(d.join("gantry.toml"), "").unwrap();
        std::fs::write(d.join("conf.d/10-typo.toml"), "[storage.kv]\nbakend = \"valkey\"\n").unwrap();
        let err = Config::load(Some(&d.join("gantry.toml"))).unwrap_err().to_string();
        assert!(err.contains("10-typo.toml") && err.contains("bakend"), "{err}");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_read_only_when_it_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("keyfile");
        let key = d.join("secret.key");
        std::fs::write(&key, "c2VjcmV0LXNlY3JldC1zZWNyZXQtc2VjcmV0LTMyYiE=\n").unwrap();
        std::fs::write(d.join("gantry.toml"), format!("[security]\nsecret_key_file = {:?}\n", key)).unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = Config::load(Some(&d.join("gantry.toml"))).unwrap_err().to_string();
        assert!(err.contains("readable by every user"), "{err}");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o640)).unwrap();
        let c = Config::load(Some(&d.join("gantry.toml"))).unwrap();
        assert_eq!(c.security.secret_key, "c2VjcmV0LXNlY3JldC1zZWNyZXQtc2VjcmV0LTMyYiE=");
        // The key never appears when the configuration is printed.
        assert!(!toml::to_string(&c).unwrap().contains("c2VjcmV0"));
        std::fs::remove_dir_all(d).unwrap();
    }
}
