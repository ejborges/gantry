//! Background jobs on a PostgreSQL queue (`FOR UPDATE SKIP LOCKED`), so any number of worker
//! processes on any number of nodes can share the work without a separate broker.
//!
//! Job kinds: `deliver` (outbound integration message), `email` (one notification message),
//! `embed` (vector index refresh), `jira_import`. Failed jobs retry with exponential backoff and end in state `failed`
//! (visible in the administration console and in `gantry_jobs_dead`).
//! Periodic duties (SLA scan, reminders, alert rules, saved-search alerts, housekeeping) run
//! under an advisory lock so one node does them.

use crate::{audit, desk, integrations, notify, search, AppState, St};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub async fn enqueue(st: &AppState, kind: &str, payload: Value) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("INSERT INTO jobs (kind, payload) VALUES ($1, $2) RETURNING id").bind(kind).bind(payload).fetch_one(&st.db).await?)
}

/// Seconds to wait before retry number `attempt` (1-based): 30 s, 2 min, 8 min, 32 min, then 2 h.
pub fn backoff_seconds(attempt: i32) -> i64 {
    (30i64 * 4i64.pow((attempt.clamp(1, 6) - 1) as u32)).min(7200)
}

async fn run(st: &St, kind: &str, payload: &Value) -> anyhow::Result<()> {
    match kind {
        "deliver" => integrations::deliver(st, payload).await,
        "email" => notify::email_job(st, payload).await,
        "embed" => search::embed_doc(st, payload).await,
        "jira_import" => integrations::jira_import(st, payload).await,
        other => anyhow::bail!("unknown job kind {other:?}"),
    }
}

/// Claim and run one job. Returns false when the queue is empty.
pub async fn run_one(st: &St) -> anyhow::Result<bool> {
    let row = sqlx::query(
        "UPDATE jobs SET state = 'running', attempts = attempts + 1, locked_by = $1, locked_at = now()
         WHERE id = (SELECT id FROM jobs WHERE state = 'queued' AND run_at <= now() ORDER BY run_at, id FOR UPDATE SKIP LOCKED LIMIT 1)
         RETURNING id, kind, payload, attempts, max_attempts",
    )
    .bind(&st.node_id)
    .fetch_optional(&st.db)
    .await?;
    let Some(row) = row else { return Ok(false) };
    let (id, kind): (i64, String) = (row.get("id"), row.get("kind"));
    let (attempts, max): (i32, i32) = (row.get("attempts"), row.get("max_attempts"));
    let started = std::time::Instant::now();
    match run(st, &kind, &row.get::<Value, _>("payload")).await {
        Ok(()) => {
            sqlx::query("UPDATE jobs SET state = 'done', finished_at = now(), last_error = NULL WHERE id = $1").bind(id).execute(&st.db).await?;
            st.metrics.jobs_done.fetch_add(1, Ordering::Relaxed);
            tracing::info!(target: "jobs", job = id, kind = %kind, attempts, duration_ms = started.elapsed().as_millis() as u64, "job done");
        }
        Err(e) => {
            let msg: String = format!("{e:#}").chars().take(2000).collect();
            if attempts >= max {
                sqlx::query("UPDATE jobs SET state = 'failed', finished_at = now(), last_error = $2 WHERE id = $1").bind(id).bind(&msg).execute(&st.db).await?;
                st.metrics.jobs_failed.fetch_add(1, Ordering::Relaxed);
                tracing::error!(target: "jobs", job = id, kind = %kind, attempts, error = %msg, "job failed permanently");
                audit::log_system(st, None, "worker", "job.failed", "job", &id.to_string(), "failure", json!({"kind": kind, "error": msg})).await;
            } else {
                sqlx::query("UPDATE jobs SET state = 'queued', run_at = now() + make_interval(secs => $2), last_error = $3 WHERE id = $1").bind(id).bind(backoff_seconds(attempts) as f64).bind(&msg).execute(&st.db).await?;
                tracing::warn!(target: "jobs", job = id, kind = %kind, attempts, error = %msg, "job failed, will retry");
            }
        }
    }
    Ok(true)
}

/// Run queued jobs until the queue is empty (used by tests and `gantry worker --once`).
pub async fn drain(st: &St) -> anyhow::Result<u32> {
    let mut n = 0;
    while run_one(st).await? {
        n += 1;
        anyhow::ensure!(n < 10_000, "job queue did not drain");
    }
    Ok(n)
}

/// Duties that must happen regularly but only on one node at a time.
pub async fn periodic(st: &St) -> anyhow::Result<()> {
    let mut tx = st.db.begin().await?;
    let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(1195462500)").fetch_one(&mut *tx).await?;
    if !got {
        return Ok(());
    }
    let breached = desk::sla_scan(st).await?;
    if breached > 0 {
        tracing::info!(target: "jobs", breached, "service-level scan");
    }
    // Each of these logs its own failure and never stops the others.
    match notify::check_alerts(st).await {
        Ok(n) if n > 0 => tracing::info!(target: "jobs", changed = n, "alert rules changed state"),
        Ok(_) => {}
        Err(e) => tracing::error!(target: "jobs", error = %e, "alert rules could not be checked"),
    }
    // Reminders go out in the hour people chose; the per-item claim makes repeats harmless.
    match notify::send_reminders(st).await {
        Ok(n) if n > 0 => tracing::info!(target: "jobs", people = n, "reminders sent"),
        Ok(_) => {}
        Err(e) => tracing::error!(target: "jobs", error = %e, "reminders could not be sent"),
    }
    if let Err(e) = crate::planning::saved_search_alerts(st).await {
        tracing::error!(target: "jobs", error = %e, "saved-search alerts could not be checked");
    }
    // Jobs whose worker died mid-run go back to the queue.
    let rescued = sqlx::query("UPDATE jobs SET state = 'queued', run_at = now() WHERE state = 'running' AND locked_at < now() - interval '15 minutes'").execute(&st.db).await?.rows_affected();
    if rescued > 0 {
        tracing::warn!(target: "jobs", rescued, "requeued jobs from a worker that stopped responding");
    }
    sqlx::query("DELETE FROM sessions WHERE expires_at < now() OR last_seen_at < now() - make_interval(mins => $1)").bind(st.cfg.auth.session_idle_minutes as i32).execute(&st.db).await?;
    sqlx::query("DELETE FROM oidc_states WHERE created_at < now() - interval '15 minutes'").execute(&st.db).await?;
    sqlx::query("DELETE FROM kv WHERE expires_at < now()").execute(&st.db).await?;
    sqlx::query("DELETE FROM jobs WHERE state = 'done' AND finished_at < now() - interval '7 days'").execute(&st.db).await?;
    sqlx::query("DELETE FROM deliveries WHERE at < now() - interval '30 days'").execute(&st.db).await?;
    sqlx::query("DELETE FROM notifications WHERE created_at < now() - interval '90 days'").execute(&st.db).await?;
    sqlx::query("DELETE FROM saml_requests WHERE created_at < now() - interval '15 minutes'").execute(&st.db).await?;
    sqlx::query("DELETE FROM saml_assertions WHERE expires_at < now()").execute(&st.db).await?;
    sqlx::query("DELETE FROM rt_messages WHERE at < now() - interval '1 minute'").execute(&st.db).await?;
    sqlx::query("DELETE FROM metric_samples WHERE at < now() - interval '7 days'").execute(&st.db).await?;
    sqlx::query("DELETE FROM email_log WHERE at < now() - interval '90 days'").execute(&st.db).await?;
    sqlx::query("DELETE FROM alert_events WHERE at < now() - interval '180 days'").execute(&st.db).await?;
    sqlx::query("DELETE FROM cluster_nodes WHERE seen_at < now() - interval '7 days'").execute(&st.db).await?;
    // Working copies nobody has touched for a month are dropped; the saved content is unaffected.
    sqlx::query("DELETE FROM collab_docs WHERE updated_at < now() - interval '30 days'").execute(&st.db).await?;
    tx.commit().await?;
    Ok(())
}

/// Worker loop: `concurrency` job runners plus the periodic duties, until `shutdown` resolves.
pub async fn worker_loop(st: St, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    tracing::info!(target: "jobs", node = %st.node_id, concurrency = st.cfg.worker.concurrency, "worker started");
    let poll = Duration::from_secs(st.cfg.worker.poll_seconds.max(1));
    let mut handles = vec![];
    for _ in 0..st.cfg.worker.concurrency.max(1) {
        let (st, mut stop) = (st.clone(), shutdown.clone());
        handles.push(tokio::spawn(async move {
            loop {
                if *stop.borrow() {
                    break;
                }
                match run_one(&st).await {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(e) => tracing::error!(target: "jobs", error = %e, "job runner error"),
                }
                tokio::select! { _ = tokio::time::sleep(poll) => {}, _ = stop.changed() => {} }
            }
        }));
    }
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    let mut sample = crate::realtime::Sample::default();
    loop {
        tokio::select! {
            _ = tick.tick() => {
                // A worker on its own machine reports in too, so the status page lists it.
                if !st.hub.serving.load(Ordering::Relaxed) {
                    if let Err(e) = crate::realtime::heartbeat(&st, "worker", &mut sample).await {
                        tracing::warn!(target: "jobs", error = %e, "node heartbeat failed");
                    }
                }
                if let Err(e) = periodic(&st).await {
                    tracing::error!(target: "jobs", error = %e, "periodic duties failed");
                }
            }
            _ = shutdown.changed() => break,
        }
    }
    for h in handles {
        let _ = h.await;
    }
    tracing::info!(target: "jobs", "worker stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff_seconds(1), 30);
        assert_eq!(backoff_seconds(2), 120);
        assert_eq!(backoff_seconds(3), 480);
        assert_eq!(backoff_seconds(4), 1920);
        assert_eq!(backoff_seconds(5), 7200);
        assert_eq!(backoff_seconds(50), 7200);
        assert_eq!(backoff_seconds(0), 30);
    }
}
