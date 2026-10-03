-- Baseline: the v1 relational schema (live user_version 8) minus the search and collection tables.
-- Applied only to a fresh database. Existing v1 databases skip this and go straight to 0009.

CREATE TABLE cycles (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  key         TEXT NOT NULL UNIQUE,
  start_date  TEXT NOT NULL,
  end_date    TEXT NOT NULL,
  description TEXT,
  status      TEXT NOT NULL DEFAULT 'planned'
              CHECK (status IN ('planned','active','done','abandoned')),
  path        TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE labels (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  slug  TEXT NOT NULL UNIQUE,
  name  TEXT NOT NULL,
  color TEXT
);

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE project_statuses (
  slug         TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  color        TEXT,
  is_archived  INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
  sort_order   INTEGER NOT NULL DEFAULT 0
, description TEXT, is_hidden INTEGER NOT NULL DEFAULT 0
  CHECK (is_hidden IN (0, 1)));

CREATE TABLE "projects" (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  slug        TEXT NOT NULL UNIQUE,
  alias       TEXT NOT NULL UNIQUE
              CHECK (alias = UPPER(alias)
                     AND LENGTH(alias) BETWEEN 2 AND 6
                     AND alias GLOB '[A-Z][A-Z0-9]*'),
  name        TEXT NOT NULL,
  path        TEXT NOT NULL UNIQUE,
  status      TEXT NOT NULL DEFAULT 'active'
              REFERENCES project_statuses(slug) ON UPDATE CASCADE,
  description TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
, space_id INTEGER REFERENCES spaces(id));

CREATE TABLE spaces (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  slug        TEXT NOT NULL UNIQUE,
  name        TEXT NOT NULL,
  path        TEXT,
  status      TEXT NOT NULL DEFAULT 'active'
              REFERENCES project_statuses(slug) ON UPDATE CASCADE,
  description TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
, alias TEXT
  CHECK (alias IS NULL OR (alias = UPPER(alias)
         AND LENGTH(alias) BETWEEN 2 AND 6
         AND alias GLOB '[A-Z][A-Z0-9]*')));

CREATE TABLE task_labels (
  task_id  INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY (task_id, label_id)
);

CREATE TABLE task_statuses (
  slug        TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  color       TEXT,
  is_closed   INTEGER NOT NULL DEFAULT 0 CHECK (is_closed IN (0, 1)),
  sort_order  INTEGER NOT NULL DEFAULT 0
, description TEXT, is_hidden INTEGER NOT NULL DEFAULT 0
  CHECK (is_hidden IN (0, 1)));

CREATE TABLE "tasks" (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id  INTEGER REFERENCES projects(id),
  space_id    INTEGER REFERENCES spaces(id),
  seq         INTEGER,
  child_seq   INTEGER,
  title       TEXT NOT NULL,
  body        TEXT,
  status      TEXT NOT NULL DEFAULT 'backlog'
              REFERENCES task_statuses(slug) ON UPDATE CASCADE,
  priority    INTEGER NOT NULL DEFAULT 3 CHECK (priority BETWEEN 1 AND 5),
  "order"     TEXT,
  cycle_id    INTEGER REFERENCES cycles(id),
  parent_id   INTEGER REFERENCES tasks(id),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  closed_at   TEXT,
  deleted_at  TEXT,
  -- Exactly one owner: a project xor a space.
  CHECK ((project_id IS NULL) <> (space_id IS NULL)),
  CHECK ((parent_id IS NULL AND seq IS NOT NULL AND child_seq IS NULL)
      OR (parent_id IS NOT NULL AND seq IS NULL AND child_seq IS NOT NULL))
);

CREATE INDEX idx_projects_space ON projects(space_id);
CREATE UNIQUE INDEX idx_spaces_alias ON spaces(alias) WHERE alias IS NOT NULL;
CREATE UNIQUE INDEX idx_tasks_parent_childseq
  ON tasks(parent_id, child_seq) WHERE parent_id IS NOT NULL;
CREATE INDEX idx_tasks_project_parent_order
  ON tasks(project_id, parent_id, "order");
CREATE UNIQUE INDEX idx_tasks_project_seq
  ON tasks(project_id, seq) WHERE project_id IS NOT NULL;
CREATE INDEX idx_tasks_space_parent_order
  ON tasks(space_id, parent_id, "order");
CREATE UNIQUE INDEX idx_tasks_space_seq
  ON tasks(space_id, seq)   WHERE space_id   IS NOT NULL;
CREATE INDEX idx_tasks_status_pri
  ON tasks(project_id, status, priority, updated_at);

CREATE TRIGGER projects_alias_no_space_clash_ins
  BEFORE INSERT ON projects
  WHEN EXISTS (SELECT 1 FROM spaces WHERE alias = NEW.alias)
BEGIN
  SELECT RAISE(ABORT, 'alias already used by a space');
END;
CREATE TRIGGER projects_alias_no_space_clash_upd
  BEFORE UPDATE OF alias ON projects
  WHEN EXISTS (SELECT 1 FROM spaces WHERE alias = NEW.alias)
BEGIN
  SELECT RAISE(ABORT, 'alias already used by a space');
END;
CREATE TRIGGER spaces_alias_no_project_clash_ins
  BEFORE INSERT ON spaces
  WHEN NEW.alias IS NOT NULL
   AND EXISTS (SELECT 1 FROM projects WHERE alias = NEW.alias)
BEGIN
  SELECT RAISE(ABORT, 'alias already used by a project');
END;
CREATE TRIGGER spaces_alias_no_project_clash_upd
  BEFORE UPDATE OF alias ON spaces
  WHEN NEW.alias IS NOT NULL
   AND EXISTS (SELECT 1 FROM projects WHERE alias = NEW.alias)
BEGIN
  SELECT RAISE(ABORT, 'alias already used by a project');
END;

INSERT INTO meta (key, value) VALUES ('top_n', '10');

INSERT INTO task_statuses (slug, name, is_closed, is_hidden, sort_order, description) VALUES
  ('in_progress', 'In Progress', 0, 0, 10, 'Actively being worked on right now.'),
  ('today',       'Today',       0, 0, 20, 'Doing it today — the day''s committed work'),
  ('cycle',       'Cycle',       0, 0, 30, 'Scheduled this cycle, not today'),
  ('backlog',     'Backlog',     0, 0, 40, 'Future work; not scheduled into a cycle yet.'),
  ('parked',      'Parked',      1, 1, 50, 'Paused or deferred; revisit later.'),
  ('done',        'Done',        1, 1, 60, 'Completed; outcome shipped or accepted.');

INSERT INTO project_statuses (slug, name, is_archived, is_hidden, sort_order, description) VALUES
  ('active',   'Active',   0, 0, 0, 'Currently being worked on.'),
  ('paused',   'Paused',   0, 0, 1, 'Temporarily on hold; work may resume.'),
  ('archived', 'Archived', 1, 0, 2, 'No longer active; retained for history and reference.');
