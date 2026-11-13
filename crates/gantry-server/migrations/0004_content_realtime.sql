-- Gantry schema, version 4 (release 0.2.0): discussion threads, project and space workflows,
-- concurrent editing and the relay that carries real-time messages between nodes.

-- ───────────────────────── discussion ─────────────────────────

ALTER TABLE comments
    -- What the author referred to, resolved under the author's access when the message was
    -- written: {"mentions": [user ids], "items": [item ids], "pages": [page ids], "projects": [project ids]}.
    ADD COLUMN refs       jsonb NOT NULL DEFAULT '{}',
    ADD COLUMN deleted_at timestamptz,
    ADD COLUMN deleted_by uuid REFERENCES users ON DELETE SET NULL;

ALTER TABLE attachments ADD COLUMN comment_id uuid REFERENCES comments ON DELETE SET NULL;

-- ───────────────────────── workflows per project and per space ─────────────────────────

-- A project may replace the workflow of one of its work item types. Without a row here the
-- type's shared workflow applies.
CREATE TABLE project_workflows (
    project_id uuid NOT NULL REFERENCES projects ON DELETE CASCADE,
    type_key   text NOT NULL REFERENCES item_types (key) ON DELETE CASCADE,
    definition jsonb NOT NULL,
    updated_by uuid REFERENCES users ON DELETE SET NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, type_key)
);

-- A space may define the stages a page moves through before it is published. `stage` is the
-- key of the page's current stage; `status` stays draft or published and follows the stage.
ALTER TABLE spaces ADD COLUMN workflow jsonb;
ALTER TABLE pages ADD COLUMN stage text NOT NULL DEFAULT '';

-- ───────────────────────── concurrent editing ─────────────────────────

-- One shared text (or rich-text document) per row. `mode` says how the content is represented:
-- 'text' for Markdown source, 'rich' for the simple editor's document tree.
CREATE TABLE collab_docs (
    room       text NOT NULL,
    doc        text NOT NULL,
    mode       text NOT NULL DEFAULT 'text' CHECK (mode IN ('text', 'rich')),
    -- The connection asked to load the saved content into the empty document, and until when
    -- that request stands. Exactly one connection seeds a document.
    seed_conn  text,
    seed_until timestamptz,
    seeded     boolean NOT NULL DEFAULT false,
    -- The saved version the working copy started from (pages: version number).
    base       bigint NOT NULL DEFAULT 0,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (room, doc)
);

-- Incremental changes in arrival order. Clients replay them to reach the current state;
-- a connected editor periodically replaces a run of them with one merged update.
CREATE TABLE collab_updates (
    id         bigserial PRIMARY KEY,
    room       text NOT NULL,
    doc        text NOT NULL,
    data       bytea NOT NULL,
    author_id  uuid REFERENCES users ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (room, doc) REFERENCES collab_docs ON DELETE CASCADE
);
CREATE INDEX collab_updates_doc ON collab_updates (room, doc, id);

-- ───────────────────────── real-time relay ─────────────────────────

-- Messages for browsers connected to other nodes. A node inserts a row and sends NOTIFY with
-- its id; every node delivers rows it did not write itself. Rows live for a minute.
CREATE UNLOGGED TABLE rt_messages (
    id      bigserial PRIMARY KEY,
    node    text NOT NULL,
    room    text NOT NULL,
    payload jsonb NOT NULL,
    at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX rt_messages_at ON rt_messages (at);

-- References (mentions, work item keys, page links) found in a description or page when it
-- was saved, kept only where the author was allowed to see the target.
ALTER TABLE items ADD COLUMN refs jsonb NOT NULL DEFAULT '{}';
ALTER TABLE pages ADD COLUMN refs jsonb NOT NULL DEFAULT '{}';

-- The display name of a status, honoring a project's own workflow for the type.
CREATE FUNCTION item_status_name(p uuid, t text, s text) RETURNS text LANGUAGE sql STABLE AS $$
    SELECT coalesce(
        (SELECT st->>'name' FROM project_workflows pw, jsonb_array_elements(pw.definition->'statuses') st
          WHERE pw.project_id = p AND pw.type_key = t AND st->>'key' = s),
        (SELECT st->>'name' FROM item_types ty JOIN workflows w ON w.key = ty.workflow_key, jsonb_array_elements(w.definition->'statuses') st
          WHERE ty.key = t AND st->>'key' = s),
        s)
$$;
