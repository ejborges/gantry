-- Gantry schema, version 3 (release 0.2.0): notification preferences, email, watchers,
-- saved searches, alert rules and the metric samples that system alerts are computed from.

ALTER TABLE notifications
    ADD COLUMN category text NOT NULL DEFAULT 'update',
    ADD COLUMN actor_id uuid REFERENCES users ON DELETE SET NULL;
CREATE INDEX notifications_unread ON notifications (user_id) WHERE read_at IS NULL;

-- A missing row means "use the workspace default for this category".
CREATE TABLE notification_prefs (
    user_id  uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    category text NOT NULL,
    in_app   boolean NOT NULL,
    email    boolean NOT NULL,
    PRIMARY KEY (user_id, category)
);

CREATE TABLE item_watchers (
    item_id    uuid NOT NULL REFERENCES items ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, user_id)
);
CREATE INDEX item_watchers_user ON item_watchers (user_id);

-- People already involved in existing work keep hearing about it.
INSERT INTO item_watchers (item_id, user_id) SELECT id, reporter_id FROM items WHERE reporter_id IS NOT NULL ON CONFLICT DO NOTHING;
INSERT INTO item_watchers (item_id, user_id) SELECT id, assignee_id FROM items WHERE assignee_id IS NOT NULL ON CONFLICT DO NOTHING;

-- One row per message handed to the mail server (or refused by it). Bodies are not kept.
CREATE TABLE email_log (
    id       bigserial PRIMARY KEY,
    at       timestamptz NOT NULL DEFAULT now(),
    user_id  uuid REFERENCES users ON DELETE SET NULL,
    to_addr  text NOT NULL,
    category text NOT NULL,
    subject  text NOT NULL,
    outcome  text NOT NULL CHECK (outcome IN ('sent', 'failed', 'skipped')),
    detail   text NOT NULL DEFAULT ''
);
CREATE INDEX email_log_at ON email_log (at DESC);

-- Reminders are sent once per item, person, kind and day.
CREATE TABLE reminders_sent (
    item_id  uuid NOT NULL REFERENCES items ON DELETE CASCADE,
    user_id  uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    kind     text NOT NULL,
    for_date date NOT NULL,
    PRIMARY KEY (item_id, user_id, kind, for_date)
);

CREATE TABLE saved_searches (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id      uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    name          text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    query         jsonb NOT NULL,
    shared        boolean NOT NULL DEFAULT false,
    -- When set, the owner is notified about items that newly match.
    alert         boolean NOT NULL DEFAULT false,
    alert_seen    uuid[] NOT NULL DEFAULT '{}',
    alert_checked timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX saved_searches_owner ON saved_searches (owner_id);

CREATE TABLE alert_rules (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    scope            text NOT NULL CHECK (scope IN ('application', 'system')),
    name             text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    kind             text NOT NULL,
    params           jsonb NOT NULL DEFAULT '{}',
    severity         text NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    -- {"in_app": true, "email": true, "roles": ["system_admin"], "addresses": [], "integrations": []}
    channels         jsonb NOT NULL DEFAULT '{}',
    enabled          boolean NOT NULL DEFAULT true,
    cooldown_minutes integer NOT NULL DEFAULT 60 CHECK (cooldown_minutes BETWEEN 1 AND 10080),
    notify_resolved  boolean NOT NULL DEFAULT true,
    state            text NOT NULL DEFAULT 'ok' CHECK (state IN ('ok', 'firing')),
    last_value       double precision,
    last_checked_at  timestamptz,
    last_fired_at    timestamptz,
    builtin          boolean NOT NULL DEFAULT false,
    created_by       uuid REFERENCES users ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE alert_events (
    id      bigserial PRIMARY KEY,
    rule_id uuid NOT NULL REFERENCES alert_rules ON DELETE CASCADE,
    at      timestamptz NOT NULL DEFAULT now(),
    state   text NOT NULL CHECK (state IN ('firing', 'resolved')),
    value   double precision,
    message text NOT NULL
);
CREATE INDEX alert_events_rule ON alert_events (rule_id, at DESC);
CREATE INDEX alert_events_at ON alert_events (at DESC);

-- Per-node counters, one row per node, name and minute. Kept for a week.
CREATE TABLE metric_samples (
    at    timestamptz NOT NULL,
    node  text NOT NULL,
    name  text NOT NULL,
    value double precision NOT NULL
);
CREATE INDEX metric_samples_name_at ON metric_samples (name, at DESC);

-- Sensible defaults an administrator can edit, turn off or delete.
INSERT INTO alert_rules (scope, name, kind, params, severity, channels, cooldown_minutes, builtin) VALUES
  ('system', 'Server errors', 'http_error_rate', '{"window_minutes": 10, "threshold_percent": 5, "min_requests": 50}', 'critical', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 60, true),
  ('system', 'Slow responses', 'http_latency', '{"window_minutes": 10, "threshold_ms": 1500, "min_requests": 50}', 'warning', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 120, true),
  ('system', 'Background job backlog', 'job_backlog', '{"threshold": 500}', 'warning', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 120, true),
  ('system', 'Background jobs failing', 'job_failures', '{"window_minutes": 60, "threshold": 5}', 'warning', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 120, true),
  ('system', 'Database connections nearly exhausted', 'db_pool', '{"threshold_percent": 90}', 'warning', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 60, true),
  ('system', 'A storage or search component is down', 'component_down', '{}', 'critical', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 30, true),
  ('system', 'Audit log could not be written', 'audit_failures', '{"window_minutes": 10, "threshold": 1}', 'critical', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 30, true),
  ('system', 'Repeated failed sign-ins', 'security_events', '{"window_minutes": 15, "threshold": 20, "kinds": ["auth.login_failed", "auth.lockout", "auth.login_throttled"]}', 'warning', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 60, true),
  ('system', 'Critical security event', 'security_events', '{"window_minutes": 5, "threshold": 1, "severity": "critical"}', 'critical', '{"in_app": true, "email": true, "roles": ["system_admin"]}', 15, true),
  ('application', 'Integration deliveries failing', 'integration_failures', '{"window_minutes": 60, "threshold": 5}', 'warning', '{"in_app": true, "email": true, "roles": ["app_admin"]}', 120, true),
  ('application', 'Service-level targets missed', 'sla_breaches', '{"window_minutes": 60, "threshold": 3}', 'warning', '{"in_app": true, "email": false, "roles": ["app_admin"]}', 120, true),
  ('application', 'Requests waiting without an owner', 'unassigned_requests', '{"older_than_minutes": 240, "threshold": 1}', 'info', '{"in_app": true, "email": false, "roles": ["app_admin"]}', 240, true);
