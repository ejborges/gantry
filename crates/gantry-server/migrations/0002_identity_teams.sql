-- Gantry schema, version 2 (release 0.2.0): richer accounts, AI agent rules, SAML, teams.

-- American English status key. Only workflows shipped in packs are touched; a workflow an
-- administrator wrote keeps the keys it was given.
UPDATE items i SET status = 'canceled'
  FROM item_types t JOIN workflows w ON w.key = t.workflow_key
 WHERE t.key = i.type_key AND w.source <> 'custom' AND i.status = 'cancelled';
UPDATE item_history h SET old = '"canceled"'::jsonb
 WHERE h.field = 'status' AND h.old = '"cancelled"'::jsonb
   AND EXISTS (SELECT 1 FROM items i WHERE i.id = h.item_id AND i.status <> 'cancelled');
UPDATE item_history h SET new = '"canceled"'::jsonb
 WHERE h.field = 'status' AND h.new = '"cancelled"'::jsonb
   AND EXISTS (SELECT 1 FROM items i WHERE i.id = h.item_id AND i.status <> 'cancelled');
UPDATE workflows SET definition = replace(definition::text, '"cancelled"', '"canceled"')::jsonb
 WHERE source <> 'custom' AND definition::text LIKE '%"cancelled"%';

-- ───────────────────────── accounts ─────────────────────────

ALTER TABLE users
    ADD COLUMN handle            text,
    ADD COLUMN timezone          text NOT NULL DEFAULT 'auto' CHECK (length(timezone) BETWEEN 1 AND 64),
    ADD COLUMN org_name          text NOT NULL DEFAULT '' CHECK (length(org_name) <= 200),
    ADD COLUMN org_code          text NOT NULL DEFAULT '' CHECK (length(org_code) <= 40),
    ADD COLUMN job_title         text NOT NULL DEFAULT '' CHECK (length(job_title) <= 200),
    ADD COLUMN presence          text NOT NULL DEFAULT 'available' CHECK (presence IN ('available', 'away', 'busy', 'out_of_office')),
    ADD COLUMN status_message    text NOT NULL DEFAULT '' CHECK (length(status_message) <= 140),
    ADD COLUMN status_until      timestamptz,
    ADD COLUMN avatar_blob       text,
    ADD COLUMN avatar_type       text,
    ADD COLUMN avatar_updated_at timestamptz,
    -- AI agent accounts only. NULL on people.
    ADD COLUMN agent_policy      jsonb,
    ADD COLUMN description       text NOT NULL DEFAULT '' CHECK (length(description) <= 2000),
    ADD COLUMN owner_id          uuid REFERENCES users ON DELETE SET NULL;

-- Mention handles: unique, lowercase, derived once from the email address.
WITH base AS (
    SELECT id, coalesce(nullif(left(regexp_replace(lower(split_part(email, '@', 1)), '[^a-z0-9._-]', '', 'g'), 30), ''), 'user') AS h, created_at
      FROM users
), numbered AS (
    SELECT id, h, row_number() OVER (PARTITION BY h ORDER BY created_at, id) AS n FROM base
)
UPDATE users u SET handle = CASE WHEN n.n = 1 THEN n.h ELSE n.h || n.n END FROM numbered n WHERE n.id = u.id;
UPDATE users SET handle = 'u' || handle WHERE handle !~ '^[a-z0-9]';
ALTER TABLE users ALTER COLUMN handle SET NOT NULL;
ALTER TABLE users ADD CONSTRAINT users_handle_format CHECK (handle ~ '^[a-z0-9][a-z0-9._-]{0,38}$');
CREATE UNIQUE INDEX users_handle_key ON users (handle);

-- Accounts created without a handle (single sign-on, SCIM, the command line) get one here.
CREATE FUNCTION users_default_handle() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    base text;
    candidate text;
    n integer := 1;
BEGIN
    IF NEW.handle IS NOT NULL AND NEW.handle <> '' THEN
        RETURN NEW;
    END IF;
    base := left(regexp_replace(lower(split_part(NEW.email, '@', 1)), '[^a-z0-9._-]', '', 'g'), 30);
    IF base = '' OR base !~ '^[a-z0-9]' THEN
        base := left('u' || base, 30);
    END IF;
    candidate := base;
    WHILE EXISTS (SELECT 1 FROM users WHERE handle = candidate) LOOP
        n := n + 1;
        candidate := base || n;
    END LOOP;
    NEW.handle := candidate;
    RETURN NEW;
END $$;
CREATE TRIGGER users_handle BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION users_default_handle();

-- Agents that existed before this release keep what they could do. New agents start with nothing.
UPDATE users SET agent_policy = '{"mode": "unrestricted"}'::jsonb WHERE kind = 'agent';

-- ───────────────────────── SAML ─────────────────────────

ALTER TABLE auth_providers DROP CONSTRAINT auth_providers_kind_check;
ALTER TABLE auth_providers ADD CONSTRAINT auth_providers_kind_check CHECK (kind IN ('oidc', 'saml'));
ALTER TABLE auth_providers ADD COLUMN config jsonb NOT NULL DEFAULT '{}';
ALTER TABLE auth_providers ALTER COLUMN client_id SET DEFAULT '';

CREATE TABLE saml_requests (
    id          text PRIMARY KEY,
    provider_id uuid NOT NULL REFERENCES auth_providers ON DELETE CASCADE,
    relay_state text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- Assertions already accepted, kept until they expire, so a captured response cannot be replayed.
CREATE TABLE saml_assertions (
    provider_id  uuid NOT NULL REFERENCES auth_providers ON DELETE CASCADE,
    assertion_id text NOT NULL,
    expires_at   timestamptz NOT NULL,
    PRIMARY KEY (provider_id, assertion_id)
);

-- ───────────────────────── teams ─────────────────────────

CREATE TABLE teams (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '' CHECK (length(description) <= 5000),
    parent_id   uuid REFERENCES teams ON DELETE SET NULL,
    archived    boolean NOT NULL DEFAULT false,
    created_by  uuid REFERENCES users ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX teams_name_key ON teams (lower(name)) WHERE NOT archived;
CREATE INDEX teams_parent ON teams (parent_id);

-- A team is defined by its positions. A position that ever had someone in it is closed,
-- not deleted, so the history on people's profiles stays intact.
CREATE TABLE team_positions (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    team_id     uuid NOT NULL REFERENCES teams ON DELETE CASCADE,
    title       text NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '' CHECK (length(description) <= 2000),
    is_lead     boolean NOT NULL DEFAULT false,
    sort        integer NOT NULL DEFAULT 0,
    closed      boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX team_positions_team ON team_positions (team_id, sort);

CREATE TABLE position_assignments (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    position_id uuid NOT NULL REFERENCES team_positions ON DELETE CASCADE,
    user_id     uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    started_on  date NOT NULL DEFAULT current_date,
    ended_on    date,
    created_by  uuid REFERENCES users ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    CHECK (ended_on IS NULL OR ended_on >= started_on)
);
CREATE INDEX position_assignments_position ON position_assignments (position_id);
CREATE INDEX position_assignments_user ON position_assignments (user_id, started_on);
-- One open assignment per person and position.
CREATE UNIQUE INDEX position_assignments_open ON position_assignments (position_id, user_id) WHERE ended_on IS NULL;

-- ───────────────────────── cluster membership ─────────────────────────

-- Every server and worker process reports in once a minute. Used for the status page and to
-- decide whether real-time messages need to be relayed to other nodes.
CREATE TABLE cluster_nodes (
    node       text PRIMARY KEY,
    role       text NOT NULL,
    version    text NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    seen_at    timestamptz NOT NULL DEFAULT now()
);

-- ───────────────────────── how a person appears in API responses ─────────────────────────

-- One definition for every place an account is shown (assignee, reporter, author, mention,
-- member lists), so the client can always tell a person from an AI agent and show the same
-- avatar and status. A status with an end time lapses on its own.
CREATE FUNCTION person_json(u users) RETURNS jsonb LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN u.id IS NULL THEN NULL ELSE jsonb_build_object(
        'id', u.id, 'name', u.name, 'kind', u.kind, 'handle', u.handle, 'active', u.active,
        'job_title', u.job_title,
        'avatar', '/api/v1/users/' || u.id || '/avatar?v=' || coalesce(floor(extract(epoch FROM u.avatar_updated_at))::bigint, 0),
        'presence', CASE WHEN u.status_until IS NOT NULL AND u.status_until < now() THEN 'available' ELSE u.presence END,
        'status_message', CASE WHEN u.status_until IS NOT NULL AND u.status_until < now() THEN '' ELSE u.status_message END) END
$$;
