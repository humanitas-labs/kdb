-- Customizable project & task statuses.
-- Replaces the hardcoded CHECK constraints with FK references to new lookup tables.
-- See .issues/iss-XXXX-customizable-statuses.md.

PRAGMA foreign_keys = OFF;

CREATE TABLE project_statuses (
  slug         TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  description  TEXT,
  color        TEXT,
  is_archived  INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1)),
  sort_order   INTEGER NOT NULL DEFAULT 0
);

INSERT INTO project_statuses (slug, name, description, color, is_archived, sort_order) VALUES
  ('active',   'Active',   'Currently being worked on.',                              NULL, 0, 0),
  ('paused',   'Paused',   'Temporarily on hold; work may resume.',                   NULL, 0, 1),
  ('archived', 'Archived', 'No longer active; retained for history and reference.',   NULL, 1, 2);

CREATE TABLE task_statuses (
  slug        TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  description TEXT,
  color       TEXT,
  is_closed   INTEGER NOT NULL DEFAULT 0 CHECK (is_closed IN (0, 1)),
  sort_order  INTEGER NOT NULL DEFAULT 0
);

INSERT INTO task_statuses (slug, name, description, color, is_closed, sort_order) VALUES
  ('backlog',     'Backlog',     'Future work; not scheduled into a cycle yet.',                 NULL, 0, 0),
  ('cycle',       'Cycle',       'Scheduled into the current cycle but not started yet.',        NULL, 0, 1),
  ('in_progress', 'In Progress', 'Actively being worked on right now.',                          NULL, 0, 2),
  ('parked',      'Parked',      'Paused or deferred; revisit later.',                           NULL, 1, 3),
  ('done',        'Done',        'Completed; outcome shipped or accepted.',                      NULL, 1, 4);

-- Rebuild projects with an FK on status instead of a CHECK constraint.
CREATE TABLE projects_new (
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
);

INSERT INTO projects_new (id, slug, alias, name, path, status, description, created_at, updated_at)
  SELECT id, slug, alias, name, path, status, description, created_at, updated_at
    FROM projects;

DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;

-- Rebuild tasks with an FK on status instead of a CHECK constraint.
-- Subtasks (parent_id IS NOT NULL) hold a sibling-local `child_seq`
-- and a NULL `seq`; top-level tasks hold `seq` and a NULL `child_seq`.
-- Existing children must be backfilled with `child_seq` and have their
-- `seq` cleared before this migration runs (or run the equivalent SQL
-- manually on legacy DBs).
-- A task belongs to exactly one owner — a project OR a space (XOR over
-- (project_id, space_id)). `spaces` is created later (0007); with FK off,
-- referencing it here at CREATE time is fine (SQLite defers FK resolution).
CREATE TABLE tasks_new (
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
  CHECK ((project_id IS NULL) <> (space_id IS NULL)),
  CHECK ((parent_id IS NULL AND seq IS NOT NULL AND child_seq IS NULL)
      OR (parent_id IS NOT NULL AND seq IS NULL AND child_seq IS NOT NULL))
);

INSERT INTO tasks_new (id, project_id, space_id, seq, child_seq, title, body, status, priority,
                       "order", cycle_id, parent_id, created_at, updated_at, closed_at, deleted_at)
  SELECT id, project_id, NULL, seq, NULL, title, body, status, priority, "order",
         cycle_id, parent_id, created_at, updated_at, closed_at, NULL
    FROM tasks;

DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

-- Top-level seq is unique per owner. SQLite treats NULLs as distinct, so a
-- combined UNIQUE(project_id, seq) can't hold across both owners — use a
-- partial unique index per owner instead.
CREATE UNIQUE INDEX idx_tasks_project_seq
  ON tasks(project_id, seq) WHERE project_id IS NOT NULL;
CREATE UNIQUE INDEX idx_tasks_space_seq
  ON tasks(space_id, seq)   WHERE space_id   IS NOT NULL;
CREATE UNIQUE INDEX idx_tasks_parent_childseq
  ON tasks(parent_id, child_seq) WHERE parent_id IS NOT NULL;

CREATE INDEX idx_tasks_status_pri
  ON tasks(project_id, status, priority, updated_at);
CREATE INDEX idx_tasks_project_parent_order
  ON tasks(project_id, parent_id, "order");
CREATE INDEX idx_tasks_space_parent_order
  ON tasks(space_id, parent_id, "order");

PRAGMA foreign_keys = ON;
