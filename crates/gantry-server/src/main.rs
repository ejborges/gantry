//! `gantry` command line: run the server or worker, migrate the schema, and administer an installation.

use clap::{Parser, Subcommand};
use gantry::{audit, auth, config::Config, crypto, jobs, packs, search, AppState};
use serde_json::json;
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gantry", version, about = "Gantry: project management, issue tracking, service desk, knowledge base and wiki")]
struct Cli {
    /// Configuration file (default: $GANTRY_CONFIG, then /etc/gantry/gantry.toml)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the HTTP server (API, web application, manual, MCP endpoint)
    Serve,
    /// Run a background worker (only needed when worker.embedded = false)
    Worker {
        /// Process the queue once and exit
        #[arg(long)]
        once: bool,
    },
    /// Create or upgrade the database schema and built-in content
    Migrate,
    /// Check configuration, database, schema and storage, and report problems
    Doctor,
    /// Rebuild the search index (and queue embeddings) for all items and pages
    Reindex,
    /// Manage administrator accounts
    Admin {
        #[command(subcommand)]
        cmd: AdminCmd,
    },
    /// Configuration helpers
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Audit log tools
    Audit {
        #[command(subcommand)]
        cmd: AuditCmd,
    },
    /// Add-on packs
    Pack {
        #[command(subcommand)]
        cmd: PackCmd,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    /// Create a system administrator. The password is read from standard input, or generated when --generate is given.
    Create {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        /// Generate a random password and print it once
        #[arg(long)]
        generate: bool,
    },
    /// Set a new password for an existing account (read from standard input) and unlock it
    ResetPassword {
        #[arg(long)]
        email: String,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print a new value for security.secret_key
    GenerateKey,
    /// Validate the configuration file and print the effective settings (secrets omitted)
    Check,
    /// Print the built-in catalog as JSON: API routes, permissions, roles, events, MCP tools, defaults
    Reference,
}

#[derive(Subcommand)]
enum AuditCmd {
    /// Recompute the audit log hash chain; exits non-zero if history was altered
    Verify,
}

#[derive(Subcommand)]
enum PackCmd {
    /// List add-on packs found on disk and whether they are installed
    List,
    /// Install or update an add-on pack by id
    Install { id: String },
}

fn init_logging(cfg: &Config) {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_new(&cfg.logging.level).unwrap_or_else(|_| EnvFilter::new("info"));
    if cfg.logging.format == "json" {
        fmt().json().flatten_event(true).with_current_span(false).with_span_list(false).with_env_filter(filter).with_writer(std::io::stderr).init();
    } else {
        fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    }
}

fn read_password() -> anyhow::Result<String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    let p = s.trim_end_matches(['\r', '\n']).to_string();
    anyhow::ensure!(!p.is_empty(), "no password was supplied on standard input");
    Ok(p)
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutdown signal received");
}

async fn serve(cfg: Config) -> anyhow::Result<()> {
    let db = gantry::connect_db(&cfg).await?;
    anyhow::ensure!(gantry::schema_is_current(&db).await, "the database schema is missing or out of date; run `gantry migrate` first");
    let bind: SocketAddr = cfg.server.bind.parse().map_err(|e| anyhow::anyhow!("server.bind: {e}"))?;
    if cfg.server.public_url.is_empty() {
        tracing::warn!("server.public_url is not set: links in notifications will be relative and cookies will not be marked Secure");
    }
    let st = AppState::init(cfg, db).await?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let worker = st.cfg.worker.embedded.then(|| tokio::spawn(jobs::worker_loop(st.clone(), stop_rx)));
    let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| anyhow::anyhow!("cannot listen on {bind}: {e}"))?;
    tracing::info!(version = gantry::VERSION, %bind, node = %st.node_id, search = st.stores.search.name(), blobs = st.stores.blob.name(), "gantry server started");
    audit::log_system(&st, None, "system", "system.start", "node", &st.node_id, "success", json!({"version": gantry::VERSION})).await;
    axum::serve(listener, gantry::http::build_router(st.clone()).into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown({
        let st = st.clone();
        async move {
            shutdown_signal().await;
            // Long-lived connections would otherwise hold the shutdown open.
            st.hub.close_all();
        }
    })
    .await?;
    let _ = stop_tx.send(true);
    if let Some(w) = worker {
        let _ = w.await;
    }
    audit::log_system(&st, None, "system", "system.stop", "node", &st.node_id, "success", json!({})).await;
    tracing::info!("gantry server stopped");
    Ok(())
}

async fn doctor(cfg: Config) -> anyhow::Result<()> {
    let mut failures = 0;
    let mut report = |name: &str, r: anyhow::Result<String>| match r {
        Ok(detail) => println!("  ok    {name}: {detail}"),
        Err(e) => {
            failures += 1;
            println!("  FAIL  {name}: {e}");
        }
    };
    println!("Gantry {} doctor", gantry::VERSION);
    report("configuration", cfg.validate().map(|_| "valid".into()));
    report("secret key", crypto::SecretBox::new(&cfg.security.secret_key).map(|_| "set".into()));
    report("public URL", if cfg.server.public_url.starts_with("https://") { Ok(cfg.server.public_url.clone()) } else { Err(anyhow::anyhow!("server.public_url should be an https:// URL in production (currently {:?})", cfg.server.public_url)) });
    report("web application", if cfg.server.web_root.join("index.html").is_file() { Ok(cfg.server.web_root.display().to_string()) } else { Err(anyhow::anyhow!("no index.html in {}", cfg.server.web_root.display())) });
    match gantry::connect_db(&cfg).await {
        Ok(db) => {
            let v: String = sqlx::query_scalar("SHOW server_version").fetch_one(&db).await?;
            report("database", Ok(format!("PostgreSQL {v}")));
            let current = gantry::schema_is_current(&db).await;
            report("schema", if current { Ok("up to date".into()) } else { Err(anyhow::anyhow!("out of date; run `gantry migrate`")) });
            if current && !cfg.security.secret_key.is_empty() {
                match AppState::init(cfg.clone(), db.clone()).await {
                    Ok(st) => {
                        report("file storage", st.stores.blob.health().await.map(|_| st.stores.blob.name().to_string()));
                        report("search", st.stores.search.health().await.map(|_| st.stores.search.name().to_string()));
                        report("cache", st.stores.kv.set("doctor:probe", "1", 10).await.map(|_| st.stores.kv.name().to_string()));
                        report("email", gantry::notify::check_mail_server(&st).await);
                        let admins: i64 = sqlx::query_scalar("SELECT count(DISTINCT b.user_id) FROM role_bindings b JOIN roles r ON r.id = b.role_id JOIN users u ON u.id = b.user_id WHERE r.key = 'system_admin' AND u.active").fetch_one(&db).await?;
                        report("administrators", if admins > 0 { Ok(format!("{admins} active")) } else { Err(anyhow::anyhow!("none; create one with `gantry admin create`")) });
                        let chain = audit::verify_chain(&db).await?;
                        report("audit log", if chain.intact { Ok(format!("{} entries, chain intact", chain.entries)) } else { Err(anyhow::anyhow!("hash chain broken at entry {:?}", chain.first_broken_id)) });
                    }
                    Err(e) => report("storage backends", Err(e)),
                }
            }
        }
        Err(e) => report("database", Err(e)),
    }
    anyhow::ensure!(failures == 0, "{failures} check(s) failed");
    println!("All checks passed.");
    Ok(())
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    if let Cmd::Config { cmd: ConfigCmd::GenerateKey } = &cli.cmd {
        println!("{}", crypto::SecretBox::generate_key());
        return Ok(());
    }
    if let Cmd::Config { cmd: ConfigCmd::Reference } = &cli.cmd {
        println!("{}", serde_json::to_string_pretty(&gantry::reference::catalog())?);
        return Ok(());
    }
    let cfg = Config::load(cli.config.as_deref())?;
    match cli.cmd {
        Cmd::Serve => {
            init_logging(&cfg);
            serve(cfg).await
        }
        Cmd::Worker { once } => {
            init_logging(&cfg);
            let st = AppState::init(cfg.clone(), gantry::connect_db(&cfg).await?).await?;
            if once {
                jobs::periodic(&st).await?;
                let n = jobs::drain(&st).await?;
                tracing::info!(jobs = n, "queue drained");
                return Ok(());
            }
            let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
            let handle = tokio::spawn(jobs::worker_loop(st, stop_rx));
            shutdown_signal().await;
            let _ = stop_tx.send(true);
            let _ = handle.await;
            Ok(())
        }
        Cmd::Migrate => {
            init_logging(&cfg);
            let db = gantry::connect_db(&cfg).await?;
            gantry::migrate(&db).await?;
            println!("Schema is up to date.");
            // Add-on packages only place files; this is where their content reaches the database.
            let (synced, failed) = packs::sync_addons(&db, &cfg.addons.dir).await;
            for (id, why) in &failed {
                eprintln!("Add-on {id} was skipped: {why}");
            }
            // Installing or updating an add-on changes what the system offers, so it is audited.
            // Runs that change nothing (every ordinary service start) stay silent.
            let changed: Vec<_> = synced.iter().filter(|s| s.changed()).collect();
            if !changed.is_empty() {
                let st = AppState::init(cfg.clone(), db.clone()).await?;
                for s in changed {
                    println!("Add-on {} {} is installed.", s.pack.id, s.pack.version);
                    audit::log_system(&st, None, "cli", "pack.install", "pack", &s.pack.id, "success", json!({"version": s.pack.version, "previous": s.previous, "via": "migrate"})).await;
                }
            }
            Ok(())
        }
        Cmd::Doctor => doctor(cfg).await,
        Cmd::Reindex => {
            init_logging(&cfg);
            let st = AppState::init(cfg.clone(), gantry::connect_db(&cfg).await?).await?;
            let (items, pages) = search::reindex_all(&st).await?;
            println!("Indexed {items} work items and {pages} pages.");
            Ok(())
        }
        Cmd::Admin { cmd } => {
            let db = gantry::connect_db(&cfg).await?;
            let st = AppState::init(cfg.clone(), db.clone()).await?;
            match cmd {
                AdminCmd::Create { email, name, generate } => {
                    let email = email.trim().to_lowercase();
                    let password = if generate { crypto::random_token(18) } else { read_password()? };
                    auth::validate_password(&st, &password).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let hash = crypto::hash_password(&password);
                    let id: uuid::Uuid = sqlx::query_scalar("INSERT INTO users (email, name, password_hash) VALUES ($1, $2, $3) RETURNING id").bind(&email).bind(name.trim()).bind(hash).fetch_one(&db).await.map_err(|e| anyhow::anyhow!("could not create the account (does it already exist?): {e}"))?;
                    sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type) SELECT id, $1, 'global' FROM roles WHERE key = 'system_admin'").bind(id).execute(&db).await?;
                    audit::log_system(&st, None, "cli", "user.create", "user", &id.to_string(), "success", json!({"email": email, "role": "system_admin"})).await;
                    println!("Created system administrator {email}.");
                    if generate {
                        println!("Password (shown once; change it after first sign-in): {password}");
                    }
                }
                AdminCmd::ResetPassword { email } => {
                    let password = read_password()?;
                    auth::validate_password(&st, &password).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let id: Option<uuid::Uuid> = sqlx::query_scalar("UPDATE users SET password_hash = $2, failed_logins = 0, locked_until = NULL, updated_at = now() WHERE lower(email) = lower($1) AND kind = 'human' RETURNING id").bind(email.trim()).bind(crypto::hash_password(&password)).fetch_optional(&db).await?;
                    let id = id.ok_or_else(|| anyhow::anyhow!("no such account"))?;
                    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&db).await?;
                    audit::log_system(&st, None, "cli", "user.password_reset", "user", &id.to_string(), "success", json!({})).await;
                    println!("Password updated; existing sessions were ended.");
                }
            }
            Ok(())
        }
        Cmd::Config { cmd: ConfigCmd::Check } => {
            println!("{}", toml::to_string_pretty(&cfg)?);
            println!("# Configuration is valid. Secret values are not shown.");
            Ok(())
        }
        Cmd::Config { cmd: ConfigCmd::GenerateKey | ConfigCmd::Reference } => unreachable!("handled above"),
        Cmd::Audit { cmd: AuditCmd::Verify } => {
            let db = gantry::connect_db(&cfg).await?;
            let r = audit::verify_chain(&db).await?;
            println!("{}", serde_json::to_string(&r)?);
            anyhow::ensure!(r.intact, "audit log hash chain is broken at entry {:?}", r.first_broken_id);
            Ok(())
        }
        Cmd::Pack { cmd } => {
            let db = gantry::connect_db(&cfg).await?;
            match cmd {
                PackCmd::List => {
                    let installed: Vec<(String, String)> = sqlx::query_as("SELECT id, version FROM packs").fetch_all(&db).await?;
                    println!("core (built in): {}", installed.iter().find(|p| p.0 == "core").map(|p| p.1.as_str()).unwrap_or("not installed"));
                    for d in packs::discover(&cfg.addons.dir) {
                        match d.pack {
                            Ok(p) => println!("{} {} — {}", p.id, p.version, installed.iter().find(|i| i.0 == p.id).map(|i| format!("installed ({})", i.1)).unwrap_or_else(|| "available".into())),
                            Err(e) => println!("{} — INVALID: {e}", d.id),
                        }
                    }
                }
                PackCmd::Install { id } => {
                    let st = AppState::init(cfg.clone(), db.clone()).await?;
                    let p = packs::install_addon(&db, &cfg.addons.dir, &id).await.map_err(|e| anyhow::anyhow!("{}", e.public_message()))?;
                    audit::log_system(&st, None, "cli", "pack.install", "pack", &p.id, "success", json!({"version": p.version})).await;
                    println!("Installed {} {}.", p.id, p.version);
                }
            }
            Ok(())
        }
    }
}

/// When started by root on a machine that has a `gantry` account, become that account
/// before doing anything else. The server never needs root, and administrators can run
/// `gantry doctor` or `gantry admin …` from a root shell and still reach PostgreSQL, which
/// identifies local clients by their system account.
#[cfg(unix)]
fn drop_root_privileges() {
    // SAFETY: libc account lookups and identity changes, made before any thread is started.
    unsafe {
        if libc::geteuid() != 0 {
            return;
        }
        let name = c"gantry";
        let pw = libc::getpwnam(name.as_ptr());
        if pw.is_null() {
            return;
        }
        let (uid, gid) = ((*pw).pw_uid, (*pw).pw_gid);
        if uid == 0 {
            return;
        }
        if libc::initgroups(name.as_ptr(), gid as _) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
            eprintln!("gantry: could not switch from root to the gantry account: {}", std::io::Error::last_os_error());
            std::process::exit(1);
        }
        // Regaining root must be impossible from here on.
        if libc::setuid(0) == 0 {
            eprintln!("gantry: refusing to continue with root privileges");
            std::process::exit(1);
        }
        let home = std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
        std::env::set_var("HOME", home);
        std::env::set_var("USER", "gantry");
        std::env::set_var("LOGNAME", "gantry");
    }
}

fn main() {
    #[cfg(unix)]
    drop_root_privileges();
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("start runtime");
    if let Err(e) = rt.block_on(run(cli)) {
        eprintln!("gantry: {e:#}");
        std::process::exit(1);
    }
}
