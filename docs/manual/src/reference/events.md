---
title: Events and webhooks
summary: What Gantry emits when things happen, and how a receiver verifies a webhook.
---

## Events

{{events}}

Each event goes to the people concerned as a notification, in the application and by email according to [their choices](../user/notifications.md), and to every integration subscribed to it whose project filter matches. Internal notes on service desk requests are never sent to integrations.

`alert.firing` and `alert.resolved` are different: they are sent to the integrations selected under **Post to** in the alert, whatever events the integration subscribes to. Their `subject` is the alert's name, their `summary` says what was measured, their `actor` is the system itself (`"id": null`), and their `data` holds the alert's `rule` name, its `severity` (`info`, `warning` or `critical`) and the `kind` of measurement.

The same events drive the application's live updates over [the WebSocket](api.md#live-updates), as hints without content.

## Webhook deliveries

A *webhook* integration receives each event as an HTTP `POST` with a JSON body:

```json
{
  "id": "5f0c1a9e2b7d4c31a8e0f6b2",
  "type": "item.transitioned",
  "occurred_at": "2026-11-10T14:03:22.418Z",
  "actor": {"id": "7f19f85e-2832-4fd8-a22a-0c5b8f0e6f11", "name": "Ada Okafor"},
  "subject": "PUMP-12 Controller resets under sustained load",
  "summary": "Ada Okafor moved PUMP-12 to In review",
  "url": "https://gantry.example.org/i/PUMP-12",
  "data": {"key": "PUMP-12", "from": "in_progress", "to": "in_review"}
}
```

`id`, `type`, `occurred_at`, `actor`, `subject`, `summary` and `url` are always present. `data` depends on the event: an item's `key` and what changed, a page's ID, and so on.

### Verifying the signature

Deliveries follow the [Standard Webhooks](https://www.standardwebhooks.com/) specification. Three headers accompany each request:

| Header | Content |
| --- | --- |
| `webhook-id` | `msg_` followed by the event's `id`. A retry carries the same ID: use it to ignore duplicates. |
| `webhook-timestamp` | Unix time of sending. Refuse anything older than five minutes. |
| `webhook-signature` | `v1,` followed by the base64 HMAC-SHA256 of `{id}.{timestamp}.{body}`, keyed with the integration's signing secret. |

The secret is shown once when the integration is created, in the form `whsec_…`; the key is the base64-decoded part after the prefix. Libraries for the specification exist for most languages. By hand, in Python:

```python
import base64, hashlib, hmac, time

def verify(secret: str, headers, body: bytes) -> bool:
    key = base64.b64decode(secret.removeprefix("whsec_"))
    msg_id, ts = headers["webhook-id"], headers["webhook-timestamp"]
    if abs(time.time() - int(ts)) > 300:
        return False
    expected = base64.b64encode(hmac.new(key, f"{msg_id}.{ts}.".encode() + body, hashlib.sha256).digest()).decode()
    return any(hmac.compare_digest(expected, s.split(",", 1)[1]) for s in headers["webhook-signature"].split() if s.startswith("v1,"))
```

### Delivery guarantees

A delivery is attempted until the receiver answers with a 2xx status, with growing delays between attempts, and is then marked failed and listed under **Background jobs**. Receivers should answer quickly and be prepared to receive the same delivery twice.

Gantry refuses to deliver to private and loopback addresses unless a system administrator allows it, and does not follow redirects.

## Receiving from other systems

| Receiver | Address | Verified by |
| --- | --- | --- |
| GitHub | `/api/v1/hooks/github/{integration-id}` | `X-Hub-Signature-256` |
| GitLab | `/api/v1/hooks/gitlab/{integration-id}` | Signing token (GitLab 19.0 and newer) or `X-Gitlab-Token` |
| Slack slash command | `/api/v1/hooks/slack/{integration-id}/command` | `X-Slack-Signature` and timestamp |

The exact address for each integration is shown in the console. A call without a valid signature gets 401 and is recorded as a security event.
