-- Spaces: a named grouping of related projects.
-- See .issues/iss-0067-spaces.md.
--
-- A space sits above projects the way a project sits above tasks.
-- Membership is one-to-many (a project belongs to at most one space via
-- a nullable `space_id`); spaces are flat (no nesting) and organizational
-- only — they do not affect task ids and impose no filesystem structure
-- (`path` is an optional reference, not enforced).
--
-- Additive migration: a nullable FK column is added to `projects` via a
-- plain ALTER (no table rebuild). Existing projects keep space_id = NULL.

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
);

ALTER TABLE projects ADD COLUMN space_id INTEGER REFERENCES spaces(id);

CREATE INDEX idx_projects_space ON projects(space_id);
