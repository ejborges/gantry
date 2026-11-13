-- Gantry schema, version 5 (release 0.2.0): hierarchy above and below projects, Gantt charts,
-- saved visualizations and dashboards.

-- ───────────────────────── hierarchy ─────────────────────────

-- Levels are ordered from the top. `kind = 'container'` levels group projects (Program,
-- Initiative, Portfolio…); `kind = 'item'` levels rank work item types inside a project
-- (Epic, Story, Task, Subtask…). The project always sits between the two.
CREATE TABLE hierarchy_levels (
    id       uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind     text NOT NULL CHECK (kind IN ('container', 'item')),
    position integer NOT NULL CHECK (position BETWEEN 1 AND 20),
    name     text NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    plural   text NOT NULL CHECK (length(plural) BETWEEN 1 AND 60),
    color    text NOT NULL DEFAULT 'neutral',
    UNIQUE (kind, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX hierarchy_levels_name ON hierarchy_levels (kind, lower(name));

INSERT INTO hierarchy_levels (kind, position, name, plural, color) VALUES
    ('container', 1, 'Program', 'Programs', 'purple'),
    ('container', 2, 'Initiative', 'Initiatives', 'blue');

CREATE TABLE containers (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    level_id    uuid NOT NULL REFERENCES hierarchy_levels ON DELETE RESTRICT,
    parent_id   uuid REFERENCES containers ON DELETE SET NULL,
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '' CHECK (length(description) <= 20000),
    owner_id    uuid REFERENCES users ON DELETE SET NULL,
    status      text NOT NULL DEFAULT 'active' CHECK (status IN ('planned', 'active', 'on_hold', 'done')),
    start_date  date,
    target_date date,
    archived    boolean NOT NULL DEFAULT false,
    created_by  uuid REFERENCES users ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX containers_parent ON containers (parent_id);
CREATE INDEX containers_owner ON containers (owner_id);

ALTER TABLE projects ADD COLUMN container_id uuid REFERENCES containers ON DELETE SET NULL;
CREATE INDEX projects_container ON projects (container_id);

ALTER TABLE item_types ADD COLUMN level_id uuid REFERENCES hierarchy_levels ON DELETE SET NULL;

-- ───────────────────────── Gantt charts ─────────────────────────

CREATE TABLE gantt_charts (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    -- What the chart shows: {"project": "OPS"} | {"container": "<id>"} | {"query": {...}}.
    scope       jsonb NOT NULL,
    -- The owner's visual cues: marker lines, shaded date ranges, labeled bands around rows.
    annotations jsonb NOT NULL DEFAULT '[]',
    settings    jsonb NOT NULL DEFAULT '{}',
    shared      boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX gantt_charts_owner ON gantt_charts (owner_id);

-- ───────────────────────── analytics ─────────────────────────

CREATE TABLE visualizations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '' CHECK (length(description) <= 5000),
    library     text NOT NULL CHECK (library IN ('chartjs', 'echarts', 'plotly', 'd3', 'html')),
    -- The script that draws the visualization. It runs in a sandboxed frame in the viewer's
    -- browser and reads data through the viewer's own access; it is never run on the server.
    source      text NOT NULL CHECK (length(source) <= 200000),
    -- Named values the script reads through gantry.params (for example a project key).
    params      jsonb NOT NULL DEFAULT '{}',
    -- The built-in template this was copied from, if any.
    template    text,
    visibility  text NOT NULL DEFAULT 'private' CHECK (visibility IN ('private', 'workspace')),
    editors     uuid[] NOT NULL DEFAULT '{}',
    version     integer NOT NULL DEFAULT 1,
    archived    boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX visualizations_owner ON visualizations (owner_id) WHERE NOT archived;

CREATE TABLE dashboards (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '' CHECK (length(description) <= 5000),
    -- [{"id": "...", "kind": "visualization" | "template", "ref": "...", "title": "", "x": 0, "y": 0, "w": 6, "h": 4, "params": {}}]
    layout      jsonb NOT NULL DEFAULT '[]',
    -- Values shared by every tile, for example {"project": "OPS"}.
    params      jsonb NOT NULL DEFAULT '{}',
    visibility  text NOT NULL DEFAULT 'private' CHECK (visibility IN ('private', 'workspace')),
    editors     uuid[] NOT NULL DEFAULT '{}',
    version     integer NOT NULL DEFAULT 1,
    archived    boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX dashboards_owner ON dashboards (owner_id) WHERE NOT archived;
