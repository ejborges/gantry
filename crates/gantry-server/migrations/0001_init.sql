-- Gantry schema, version 1.
-- PostgreSQL 15 or newer. One database holds relational data, JSONB documents, the job queue,
-- the full-text index, graph edges and (portable) vectors. Dedicated engines are optional
-- and plug in behind the storage ports; see docs/manual/sysadmin/storage.md.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ───────────────────────── identity and access ─────────────────────────

CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind          text NOT NULL DEFAULT 'human' CHECK (kind IN ('human', 'agent')),
    email         text NOT NULL CHECK (length(email) BETWEEN 3 AND 320),
    name          text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    password_hash text,
    active        boolean NOT NULL DEFAULT true,
    theme         text NOT NULL DEFAULT 'system' CHECK (theme IN ('system', 'light', 'dark')),
    failed_logins integer NOT NULL DEFAULT 0,
    locked_until  timestamptz,
    last_login_at timestamptz,
    external_id   text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_email_key ON users (lower(email));

CREATE TABLE groups (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '',
    external_id text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE group_members (
    group_id uuid NOT NULL REFERENCES groups ON DELETE CASCADE,
    user_id  uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE roles (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    key         text NOT NULL UNIQUE CHECK (key ~ '^[a-z][a-z0-9_]{1,62}$'),
    name        text NOT NULL,
    scope       text NOT NULL CHECK (scope IN ('global', 'project', 'space')),
    permissions text[] NOT NULL DEFAULT '{}',
    builtin     boolean NOT NULL DEFAULT false,
    description text NOT NULL DEFAULT ''
);

CREATE TABLE role_bindings (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    role_id    uuid NOT NULL REFERENCES roles ON DELETE CASCADE,
    user_id    uuid REFERENCES users ON DELETE CASCADE,
    group_id   uuid REFERENCES groups ON DELETE CASCADE,
    scope_type text NOT NULL CHECK (scope_type IN ('global', 'project', 'space')),
    scope_id   uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((user_id IS NULL) <> (group_id IS NULL)),
    CHECK ((scope_type = 'global') = (scope_id IS NULL))
);
CREATE UNIQUE INDEX role_bindings_unique ON role_bindings
    (role_id, scope_type,
     coalesce(scope_id, '00000000-0000-0000-0000-000000000000'),
     coalesce(user_id, '00000000-0000-0000-0000-000000000000'),
     coalesce(group_id, '00000000-0000-0000-0000-000000000000'));
CREATE INDEX role_bindings_user ON role_bindings (user_id);
CREATE INDEX role_bindings_group ON role_bindings (group_id);
CREATE INDEX role_bindings_scope ON role_bindings (scope_type, scope_id);

CREATE TABLE auth_providers (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind              text NOT NULL DEFAULT 'oidc' CHECK (kind IN ('oidc')),
    name              text NOT NULL,
    issuer            text NOT NULL,
    client_id         text NOT NULL,
    client_secret_enc text NOT NULL DEFAULT '',
    scopes            text NOT NULL DEFAULT 'openid email profile',
    enabled           boolean NOT NULL DEFAULT true,
    auto_provision    boolean NOT NULL DEFAULT true,
    link_by_email     boolean NOT NULL DEFAULT true,
    allowed_domains   text[] NOT NULL DEFAULT '{}',
    default_role      text NOT NULL DEFAULT 'member',
    created_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE identities (
    provider_id uuid NOT NULL REFERENCES auth_providers ON DELETE CASCADE,
    subject     text NOT NULL,
    user_id     uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider_id, subject)
);

CREATE TABLE oidc_states (
    state         text PRIMARY KEY,
    provider_id   uuid NOT NULL REFERENCES auth_providers ON DELETE CASCADE,
    nonce         text NOT NULL,
    pkce_verifier text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash   bytea NOT NULL UNIQUE,
    user_id      uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    csrf         text NOT NULL,
    auth_method  text NOT NULL,
    ip           text NOT NULL DEFAULT '',
    user_agent   text NOT NULL DEFAULT '',
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL
);
CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE api_tokens (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    prefix       text NOT NULL UNIQUE,
    secret_hash  bytea NOT NULL,
    scopes       text[] NOT NULL DEFAULT '{read}',
    created_by   uuid REFERENCES users ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz,
    last_used_at timestamptz,
    revoked_at   timestamptz
);
CREATE INDEX api_tokens_user ON api_tokens (user_id);

CREATE TABLE settings (
    key        text PRIMARY KEY,
    value      jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    updated_by uuid REFERENCES users ON DELETE SET NULL
);

-- ───────────────────────── audit and security ─────────────────────────

CREATE TABLE audit_log (
    id          bigserial PRIMARY KEY,
    at          timestamptz NOT NULL,
    actor_id    uuid,
    actor_kind  text NOT NULL DEFAULT 'system',
    actor_label text NOT NULL DEFAULT '',
    auth_method text NOT NULL DEFAULT '',
    action      text NOT NULL,
    target_type text NOT NULL DEFAULT '',
    target_id   text NOT NULL DEFAULT '',
    scope_type  text NOT NULL DEFAULT '',
    scope_id    uuid,
    ip          text NOT NULL DEFAULT '',
    request_id  text NOT NULL DEFAULT '',
    outcome     text NOT NULL DEFAULT 'success',
    detail      jsonb NOT NULL DEFAULT '{}',
    prev_hash   bytea,
    hash        bytea NOT NULL
);
CREATE INDEX audit_log_at ON audit_log (at DESC);
CREATE INDEX audit_log_actor ON audit_log (actor_id, at DESC);
CREATE INDEX audit_log_target ON audit_log (target_type, target_id);
CREATE INDEX audit_log_action ON audit_log (action, at DESC);

-- The audit trail is append-only. Retention is an operator procedure, not an application feature.
CREATE FUNCTION audit_log_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit_log is append-only';
END $$;
CREATE TRIGGER audit_log_no_change BEFORE UPDATE OR DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_immutable();
CREATE TRIGGER audit_log_no_truncate BEFORE TRUNCATE ON audit_log
    FOR EACH STATEMENT EXECUTE FUNCTION audit_log_immutable();

CREATE TABLE security_events (
    id          bigserial PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT now(),
    kind        text NOT NULL,
    severity    text NOT NULL DEFAULT 'warning' CHECK (severity IN ('info', 'warning', 'critical')),
    actor_id    uuid,
    actor_label text NOT NULL DEFAULT '',
    ip          text NOT NULL DEFAULT '',
    request_id  text NOT NULL DEFAULT '',
    detail      jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX security_events_at ON security_events (at DESC);
CREATE INDEX security_events_kind ON security_events (kind, at DESC);

-- ───────────────────────── schemes (data-driven customization) ─────────────────────────

CREATE TABLE packs (
    id           text PRIMARY KEY,
    name         text NOT NULL,
    version      text NOT NULL,
    description  text NOT NULL DEFAULT '',
    manifest     jsonb NOT NULL,
    installed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE fields (
    key         text PRIMARY KEY CHECK (key ~ '^[a-z][a-z0-9_]{0,62}$'),
    name        text NOT NULL,
    type        text NOT NULL CHECK (type IN ('text', 'textarea', 'number', 'date', 'select', 'multiselect', 'user', 'url', 'checkbox')),
    options     jsonb NOT NULL DEFAULT '[]',
    description text NOT NULL DEFAULT '',
    source      text NOT NULL DEFAULT 'custom'
);

CREATE TABLE workflows (
    key        text PRIMARY KEY CHECK (key ~ '^[a-z][a-z0-9_]{0,62}$'),
    name       text NOT NULL,
    definition jsonb NOT NULL,
    source     text NOT NULL DEFAULT 'custom',
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE item_types (
    key          text PRIMARY KEY CHECK (key ~ '^[a-z][a-z0-9_]{0,62}$'),
    name         text NOT NULL,
    icon         text NOT NULL DEFAULT 'circle',
    color        text NOT NULL DEFAULT 'neutral',
    description  text NOT NULL DEFAULT '',
    workflow_key text NOT NULL REFERENCES workflows (key),
    fields       jsonb NOT NULL DEFAULT '[]',
    source       text NOT NULL DEFAULT 'custom',
    active       boolean NOT NULL DEFAULT true
);

CREATE TABLE project_templates (
    key         text PRIMARY KEY,
    name        text NOT NULL,
    domain      text NOT NULL DEFAULT 'General',
    kind        text NOT NULL DEFAULT 'standard',
    description text NOT NULL DEFAULT '',
    definition  jsonb NOT NULL,
    source      text NOT NULL DEFAULT 'custom'
);

CREATE TABLE page_templates (
    key         text PRIMARY KEY,
    name        text NOT NULL,
    category    text NOT NULL DEFAULT 'General',
    description text NOT NULL DEFAULT '',
    body        text NOT NULL,
    source      text NOT NULL DEFAULT 'custom'
);

-- ───────────────────────── projects and work items ─────────────────────────

CREATE TABLE projects (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    key          text NOT NULL UNIQUE CHECK (key ~ '^[A-Z][A-Z0-9]{1,9}$'),
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description  text NOT NULL DEFAULT '',
    kind         text NOT NULL DEFAULT 'standard' CHECK (kind IN ('standard', 'service_desk')),
    access       text NOT NULL DEFAULT 'private' CHECK (access IN ('private', 'internal')),
    template_key text,
    default_type text NOT NULL REFERENCES item_types (key),
    lead_id      uuid REFERENCES users ON DELETE SET NULL,
    next_number  integer NOT NULL DEFAULT 1,
    settings     jsonb NOT NULL DEFAULT '{}',
    archived     boolean NOT NULL DEFAULT false,
    created_by   uuid REFERENCES users ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE project_types (
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    type_key   text NOT NULL REFERENCES item_types (key),
    PRIMARY KEY (project_id, type_key)
);

CREATE TABLE cycles (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    name       text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    goal       text NOT NULL DEFAULT '',
    start_date date,
    end_date   date,
    state      text NOT NULL DEFAULT 'planned' CHECK (state IN ('planned', 'active', 'closed')),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE request_types (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '',
    icon        text NOT NULL DEFAULT 'help-circle',
    type_key    text NOT NULL REFERENCES item_types (key),
    form        jsonb NOT NULL DEFAULT '[]',
    position    integer NOT NULL DEFAULT 0,
    active      boolean NOT NULL DEFAULT true
);

CREATE TABLE sla_policies (
    project_id         uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    priority           smallint NOT NULL CHECK (priority BETWEEN 0 AND 4),
    response_minutes   integer NOT NULL CHECK (response_minutes > 0),
    resolution_minutes integer NOT NULL CHECK (resolution_minutes > 0),
    PRIMARY KEY (project_id, priority)
);

CREATE TABLE items (
    id                      uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id              uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    number                  integer NOT NULL,
    type_key                text NOT NULL REFERENCES item_types (key),
    title                   text NOT NULL CHECK (length(title) BETWEEN 1 AND 500),
    description             text NOT NULL DEFAULT '',
    status                  text NOT NULL,
    status_category         text NOT NULL CHECK (status_category IN ('todo', 'in_progress', 'done')),
    priority                smallint NOT NULL DEFAULT 2 CHECK (priority BETWEEN 0 AND 4),
    assignee_id             uuid REFERENCES users ON DELETE SET NULL,
    reporter_id             uuid REFERENCES users ON DELETE SET NULL,
    parent_id               uuid REFERENCES items ON DELETE SET NULL,
    labels                  text[] NOT NULL DEFAULT '{}',
    start_date              date,
    due_date                date,
    estimate                double precision,
    cycle_id                uuid REFERENCES cycles ON DELETE SET NULL,
    rank                    double precision NOT NULL DEFAULT 0,
    custom                  jsonb NOT NULL DEFAULT '{}',
    channel                 text NOT NULL DEFAULT 'web',
    request_type_id         uuid REFERENCES request_types ON DELETE SET NULL,
    first_response_at       timestamptz,
    resolved_at             timestamptz,
    sla_response_due        timestamptz,
    sla_resolution_due      timestamptz,
    sla_response_breached   boolean NOT NULL DEFAULT false,
    sla_resolution_breached boolean NOT NULL DEFAULT false,
    archived                boolean NOT NULL DEFAULT false,
    created_at              timestamptz NOT NULL DEFAULT now(),
    updated_at              timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, number)
);
CREATE INDEX items_project_status ON items (project_id, status_category, rank) WHERE NOT archived;
CREATE INDEX items_assignee ON items (assignee_id, status_category) WHERE NOT archived;
CREATE INDEX items_reporter ON items (reporter_id) WHERE NOT archived;
CREATE INDEX items_parent ON items (parent_id);
CREATE INDEX items_labels ON items USING gin (labels);
CREATE INDEX items_custom ON items USING gin (custom jsonb_path_ops);
CREATE INDEX items_sla ON items (sla_resolution_due) WHERE status_category <> 'done' AND NOT archived;

CREATE TABLE comments (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id    uuid NOT NULL REFERENCES items ON DELETE CASCADE,
    author_id  uuid REFERENCES users ON DELETE SET NULL,
    body       text NOT NULL CHECK (length(body) BETWEEN 1 AND 100000),
    internal   boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    edited_at  timestamptz
);
CREATE INDEX comments_item ON comments (item_id, created_at);

CREATE TABLE item_history (
    id       bigserial PRIMARY KEY,
    item_id  uuid NOT NULL REFERENCES items ON DELETE CASCADE,
    actor_id uuid REFERENCES users ON DELETE SET NULL,
    field    text NOT NULL,
    old      jsonb,
    new      jsonb,
    at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX item_history_item ON item_history (item_id, at);

-- Graph edges. Nodes are (type, id) pairs: item, page, commit, change_request, review.
CREATE TABLE links (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    src_type   text NOT NULL,
    src_id     text NOT NULL,
    rel        text NOT NULL CHECK (rel ~ '^[a-z][a-z_]{1,40}$'),
    dst_type   text NOT NULL,
    dst_id     text NOT NULL,
    meta       jsonb NOT NULL DEFAULT '{}',
    created_by uuid REFERENCES users ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (src_type, src_id, rel, dst_type, dst_id)
);
CREATE INDEX links_dst ON links (dst_type, dst_id);

CREATE TABLE external_refs (
    system      text NOT NULL,
    external_id text NOT NULL,
    item_id     uuid NOT NULL REFERENCES items ON DELETE CASCADE,
    url         text NOT NULL DEFAULT '',
    synced_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (system, external_id)
);
CREATE INDEX external_refs_item ON external_refs (item_id);

CREATE TABLE attachments (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_type   text NOT NULL CHECK (owner_type IN ('item', 'page')),
    owner_id     uuid NOT NULL,
    filename     text NOT NULL CHECK (length(filename) BETWEEN 1 AND 255),
    content_type text NOT NULL DEFAULT 'application/octet-stream',
    size         bigint NOT NULL,
    sha256       text NOT NULL,
    blob_key     text NOT NULL,
    uploaded_by  uuid REFERENCES users ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX attachments_owner ON attachments (owner_type, owner_id);

-- ───────────────────────── wiki and knowledge base ─────────────────────────

CREATE TABLE spaces (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    key         text NOT NULL UNIQUE CHECK (key ~ '^[A-Z][A-Z0-9]{1,9}$'),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '',
    kind        text NOT NULL DEFAULT 'wiki' CHECK (kind IN ('wiki', 'knowledge_base')),
    access      text NOT NULL DEFAULT 'private' CHECK (access IN ('private', 'internal')),
    project_id  uuid REFERENCES projects ON DELETE SET NULL,
    archived    boolean NOT NULL DEFAULT false,
    created_by  uuid REFERENCES users ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE pages (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    space_id         uuid NOT NULL REFERENCES spaces ON DELETE CASCADE,
    parent_id        uuid REFERENCES pages ON DELETE SET NULL,
    title            text NOT NULL CHECK (length(title) BETWEEN 1 AND 300),
    content          text NOT NULL DEFAULT '',
    version          integer NOT NULL DEFAULT 1,
    status           text NOT NULL DEFAULT 'published' CHECK (status IN ('draft', 'published')),
    customer_visible boolean NOT NULL DEFAULT false,
    labels           text[] NOT NULL DEFAULT '{}',
    position         integer NOT NULL DEFAULT 0,
    editor           text NOT NULL DEFAULT 'simple' CHECK (editor IN ('simple', 'advanced')),
    archived         boolean NOT NULL DEFAULT false,
    created_by       uuid REFERENCES users ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_by       uuid REFERENCES users ON DELETE SET NULL,
    updated_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX pages_space ON pages (space_id, parent_id, position) WHERE NOT archived;

CREATE TABLE page_versions (
    page_id    uuid NOT NULL REFERENCES pages ON DELETE CASCADE,
    version    integer NOT NULL,
    title      text NOT NULL,
    content    text NOT NULL,
    author_id  uuid REFERENCES users ON DELETE SET NULL,
    message    text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (page_id, version)
);

-- ───────────────────────── search, vectors, cache, queue ─────────────────────────

CREATE TABLE search_docs (
    kind             text NOT NULL,
    ref_id           uuid NOT NULL,
    project_id       uuid,
    space_id         uuid,
    label            text NOT NULL DEFAULT '',
    url              text NOT NULL DEFAULT '',
    title            text NOT NULL DEFAULT '',
    body             text NOT NULL DEFAULT '',
    customer_visible boolean NOT NULL DEFAULT false,
    updated_at       timestamptz NOT NULL DEFAULT now(),
    tsv              tsvector GENERATED ALWAYS AS (
                         setweight(to_tsvector('english', label || ' ' || title), 'A') ||
                         setweight(to_tsvector('english', left(body, 500000)), 'B')) STORED,
    PRIMARY KEY (kind, ref_id)
);
CREATE INDEX search_docs_tsv ON search_docs USING gin (tsv);
CREATE INDEX search_docs_title_trgm ON search_docs USING gin (title gin_trgm_ops);
CREATE INDEX search_docs_project ON search_docs (project_id);
CREATE INDEX search_docs_space ON search_docs (space_id);

CREATE TABLE embeddings (
    kind       text NOT NULL,
    ref_id     uuid NOT NULL,
    chunk      integer NOT NULL,
    project_id uuid,
    space_id   uuid,
    label      text NOT NULL DEFAULT '',
    url        text NOT NULL DEFAULT '',
    title      text NOT NULL DEFAULT '',
    content    text NOT NULL,
    model      text NOT NULL,
    embedding  real[] NOT NULL,
    PRIMARY KEY (kind, ref_id, chunk)
);
CREATE INDEX embeddings_project ON embeddings (project_id);
CREATE INDEX embeddings_space ON embeddings (space_id);

-- Dot product of two equal-length vectors. Stored vectors are unit length, so this is cosine similarity.
CREATE FUNCTION vec_dot(a real[], b real[]) RETURNS double precision
    LANGUAGE sql IMMUTABLE PARALLEL SAFE AS
$$ SELECT coalesce(sum(x::double precision * y::double precision), 0) FROM unnest(a, b) AS t (x, y) $$;

CREATE UNLOGGED TABLE kv (
    key        text PRIMARY KEY,
    value      text NOT NULL,
    expires_at timestamptz NOT NULL
);

CREATE TABLE jobs (
    id           bigserial PRIMARY KEY,
    kind         text NOT NULL,
    payload      jsonb NOT NULL DEFAULT '{}',
    state        text NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'running', 'done', 'failed')),
    run_at       timestamptz NOT NULL DEFAULT now(),
    attempts     integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL DEFAULT 6,
    locked_by    text,
    locked_at    timestamptz,
    last_error   text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    finished_at  timestamptz
);
CREATE INDEX jobs_ready ON jobs (run_at) WHERE state = 'queued';
CREATE INDEX jobs_state ON jobs (state, finished_at);

CREATE TABLE notifications (
    id         bigserial PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    kind       text NOT NULL,
    title      text NOT NULL,
    body       text NOT NULL DEFAULT '',
    link       text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    read_at    timestamptz
);
CREATE INDEX notifications_user ON notifications (user_id, created_at DESC);

-- ───────────────────────── integrations ─────────────────────────

CREATE TABLE integrations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind        text NOT NULL CHECK (kind ~ '^[a-z][a-z0-9_]{1,30}$'),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    enabled     boolean NOT NULL DEFAULT true,
    config      jsonb NOT NULL DEFAULT '{}',
    secrets_enc text NOT NULL DEFAULT '',
    events      text[] NOT NULL DEFAULT '{}',
    project_id  uuid REFERENCES projects ON DELETE CASCADE,
    created_by  uuid REFERENCES users ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE deliveries (
    id             bigserial PRIMARY KEY,
    integration_id uuid NOT NULL REFERENCES integrations ON DELETE CASCADE,
    direction      text NOT NULL CHECK (direction IN ('out', 'in')),
    event          text NOT NULL,
    delivery_id    text NOT NULL DEFAULT '',
    status         integer,
    outcome        text NOT NULL DEFAULT 'ok',
    detail         text NOT NULL DEFAULT '',
    at             timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX deliveries_integration ON deliveries (integration_id, at DESC);
CREATE UNIQUE INDEX deliveries_inbound_once ON deliveries (integration_id, delivery_id)
    WHERE direction = 'in' AND delivery_id <> '';
