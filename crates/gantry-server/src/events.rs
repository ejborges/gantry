//! Domain events. One call fans out to three audiences:
//!
//! * open browser tabs, as a "this changed" hint over the real-time connection;
//! * people, as notifications on the channels they chose (see `notify.rs`), limited to those
//!   who are allowed to see the thing the event is about;
//! * integrations subscribed to the event kind (Slack, Teams, webhooks), through the job queue.

use crate::notify::{self, Notice};
use crate::{authz, jobs, realtime, AppState};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

/// Every event kind the server emits. Integrations subscribe to these names (or `*`, which
/// covers everything except the `alert.*` kinds).
pub const EVENT_KINDS: &[(&str, &str)] = &[
    ("item.created", "A work item was created"),
    ("item.updated", "Fields of a work item changed"),
    ("item.assigned", "A work item was assigned"),
    ("item.transitioned", "A work item moved to another status"),
    ("comment.created", "A message was added to a work item's discussion"),
    ("mention.created", "Someone was mentioned in a discussion, a description or a page"),
    ("request.created", "A service desk request was raised through the portal"),
    ("sla.breached", "A service-level target was missed"),
    ("page.created", "A documentation page was published"),
    ("page.updated", "A published page changed"),
    ("alert.firing", "An administrator's alert rule went over its threshold"),
    ("alert.resolved", "An alert rule returned to normal"),
];

pub struct Event<'a> {
    pub kind: &'a str,
    pub project_id: Option<Uuid>,
    pub space_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub actor_name: &'a str,
    /// Short title of the thing the event is about, e.g. `OPS-12 Replace pump seal`.
    pub subject: String,
    /// Path within the web application, e.g. `/i/OPS-12`.
    pub url: String,
    /// One sentence a person can read in a chat channel.
    pub summary: String,
    pub data: Value,
    /// People to tell directly, in addition to the watchers of `item`.
    pub notify: Vec<Uuid>,
    /// The work item this is about (id and key). Its watchers are told too.
    pub item: Option<(Uuid, String)>,
    /// The page this is about (id and space key).
    pub page: Option<(Uuid, String)>,
    /// The message or change in words, quoted in email. Plain text.
    pub excerpt: String,
    /// People who are told about this in another way (for example by a mention).
    pub skip: Vec<Uuid>,
}

/// Notification category for people named in `notify`, and for watchers (if they are told at all).
fn categories(kind: &str) -> (&'static str, Option<&'static str>) {
    match kind {
        "item.created" | "item.assigned" => ("assigned", (kind == "item.assigned").then_some("update")),
        "item.transitioned" => ("status", Some("status")),
        "comment.created" => ("comment", Some("comment")),
        "mention.created" => ("mention", None),
        "request.created" => ("desk_new", None),
        "sla.breached" => ("desk_sla", None),
        _ => ("update", Some("update")),
    }
}

fn excerpt(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() > 1200 {
        format!("{}…", t.chars().take(1200).collect::<String>())
    } else {
        t.to_string()
    }
}

async fn notify_people(st: &AppState, e: &Event<'_>) -> sqlx::Result<()> {
    let (direct_cat, watcher_cat) = categories(e.kind);
    let skip: HashSet<Uuid> = e.skip.iter().copied().chain(e.actor_id).collect();
    let direct: Vec<Uuid> = e.notify.iter().copied().filter(|u| !skip.contains(u)).collect();
    let internal = e.data.get("internal").and_then(Value::as_bool).unwrap_or(false);
    let notice = |category: &'static str, link: String| Notice { category, kind: e.kind.to_string(), title: e.summary.clone(), body: e.subject.clone(), link, actor_id: e.actor_id, excerpt: excerpt(&e.excerpt), thread: e.item.as_ref().map(|i| i.0.to_string()).or(e.page.as_ref().map(|p| p.0.to_string())) };

    if let (Some((item_id, key)), Some(project_id)) = (&e.item, e.project_id) {
        let watchers: Vec<Uuid> = match watcher_cat {
            Some(_) => sqlx::query_scalar("SELECT user_id FROM item_watchers WHERE item_id = $1").bind(item_id).fetch_all(&st.db).await?.into_iter().filter(|u| !skip.contains(u) && !direct.contains(u)).collect(),
            None => vec![],
        };
        let everyone: Vec<Uuid> = direct.iter().chain(&watchers).copied().collect();
        if everyone.is_empty() {
            return Ok(());
        }
        // Only people who can open the item hear about it. An internal note stays with the agents.
        let team: HashSet<Uuid> = authz::project_audience(st, project_id, if internal { "desk.agent" } else { "project.view" }, &everyone).await?.into_iter().collect();
        notify::deliver(st, &direct.iter().copied().filter(|u| team.contains(u)).collect::<Vec<_>>(), &notice(direct_cat, e.url.clone())).await;
        if let Some(cat) = watcher_cat {
            notify::deliver(st, &watchers.iter().copied().filter(|u| team.contains(u)).collect::<Vec<_>>(), &notice(cat, e.url.clone())).await;
        }
        // The person who raised a request follows replies and status changes through the portal.
        if !internal && matches!(e.kind, "comment.created" | "item.transitioned") {
            let reporter: Option<Uuid> = sqlx::query_scalar("SELECT reporter_id FROM items WHERE id = $1").bind(item_id).fetch_optional(&st.db).await?.flatten();
            if let Some(r) = reporter.filter(|r| everyone.contains(r) && !team.contains(r)) {
                if !authz::project_audience(st, project_id, "desk.request", &[r]).await?.is_empty() {
                    notify::deliver(st, &[r], &notice(if e.kind == "comment.created" { "comment" } else { "status" }, format!("/portal/requests/{key}"))).await;
                }
            }
        }
    } else if let Some(space_id) = e.space_id {
        let readers = authz::space_audience(st, space_id, &direct).await?;
        notify::deliver(st, &readers, &notice(direct_cat, e.url.clone())).await;
    } else if let Some(project_id) = e.project_id {
        let team = authz::project_audience(st, project_id, "project.view", &direct).await?;
        notify::deliver(st, &team, &notice(direct_cat, e.url.clone())).await;
    } else {
        notify::deliver(st, &direct, &notice(direct_cat, e.url.clone())).await;
    }
    Ok(())
}

pub async fn emit(st: &AppState, e: Event<'_>) {
    tracing::debug!(target: "events", kind = e.kind, subject = %e.subject, "event");

    // Hints carry the kind and the name of the thing, never its content.
    let hint = json!({"key": e.item.as_ref().map(|i| &i.1), "page_id": e.page.as_ref().map(|p| p.0), "actor_id": e.actor_id});
    if let (Some(project_id), Some((_, key))) = (e.project_id, &e.item) {
        let project_key = key.rsplit_once('-').map(|k| k.0).unwrap_or(key);
        realtime::publish_work(st, project_id, project_key, Some(key), e.kind, hint).await;
    } else if let Some((page_id, space_key)) = &e.page {
        for room in [format!("page:{page_id}"), format!("space:{space_key}")] {
            realtime::publish(st, &room, json!({"t": "event", "room": room, "kind": e.kind, "data": hint})).await;
        }
    }

    if let Err(err) = notify_people(st, &e).await {
        tracing::error!(error = %err, kind = e.kind, "failed to notify people about an event");
    }

    // Internal notes never leave the system.
    if e.data.get("internal").and_then(Value::as_bool).unwrap_or(false) {
        return;
    }
    let targets: Result<Vec<Uuid>, _> = sqlx::query_scalar(
        "SELECT id FROM integrations WHERE enabled AND kind IN ('slack', 'teams', 'webhook')
           AND (events @> ARRAY[$1] OR events @> ARRAY['*']) AND (project_id IS NULL OR project_id = $2)",
    )
    .bind(e.kind)
    .bind(e.project_id)
    .fetch_all(&st.db)
    .await;
    match targets {
        Ok(ids) => {
            let base = st.cfg.server.public_url.trim_end_matches('/');
            for id in ids {
                let payload = json!({
                    "integration_id": id,
                    "event": {
                        "id": crate::crypto::random_hex(12), "type": e.kind, "occurred_at": chrono::Utc::now(),
                        "actor": {"id": e.actor_id, "name": e.actor_name},
                        "subject": e.subject, "summary": e.summary, "url": format!("{base}{}", e.url), "data": e.data,
                    }
                });
                if let Err(err) = jobs::enqueue(st, "deliver", payload).await {
                    tracing::error!(error = %err, integration = %id, "failed to queue delivery");
                }
            }
        }
        Err(err) => tracing::error!(error = %err, "failed to look up integrations for event"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kinds_are_unique_and_namespaced() {
        let mut seen = std::collections::HashSet::new();
        for (k, d) in EVENT_KINDS {
            assert!(seen.insert(k), "duplicate {k}");
            assert!(k.contains('.') && !d.is_empty());
        }
    }

    #[test]
    fn every_event_kind_maps_to_a_known_notification_category() {
        for (k, _) in EVENT_KINDS {
            let (direct, watchers) = categories(k);
            for c in [Some(direct), watchers].into_iter().flatten() {
                assert!(crate::settings::CATEGORIES.iter().any(|x| x.0 == c), "{k} maps to unknown category {c}");
            }
        }
        assert_eq!(categories("item.created"), ("assigned", None), "creating an item tells only its assignee");
        assert_eq!(categories("mention.created"), ("mention", None));
    }

    #[test]
    fn long_text_is_cut_for_email() {
        assert_eq!(excerpt("  hello  "), "hello");
        let long = "é".repeat(5000);
        let cut = excerpt(&long);
        assert_eq!(cut.chars().count(), 1201);
        assert!(cut.ends_with('…'));
    }
}
