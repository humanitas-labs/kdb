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

-- A space carries an optional uppercase `alias` (same shape as projects.alias)
-- so it can own tasks directly and mint ids off it. Uniqueness among spaces is
-- a partial index; cross-container uniqueness (no space/project alias clash) is
-- enforced by the triggers below, since a per-table UNIQUE can't span both.
CREATE TABLE spaces (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  slug        TEXT NOT NULL UNIQUE,
  name        TEXT NOT NULL,
  alias       TEXT CHECK (alias IS NULL OR (alias = UPPER(alias)
                     AND LENGTH(alias) BETWEEN 2 AND 6
                     AND alias GLOB '[A-Z][A-Z0-9]*')),
  path        TEXT,
  status      TEXT NOT NULL DEFAULT 'active'
              REFERENCES project_statuses(slug) ON UPDATE CASCADE,
  description TEXT,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

ALTER TABLE projects ADD COLUMN space_id INTEGER REFERENCES spaces(id);

CREATE INDEX idx_projects_space ON projects(space_id);
CREATE UNIQUE INDEX idx_spaces_alias ON spaces(alias) WHERE alias IS NOT NULL;

-- Cross-container alias uniqueness: a space alias must not collide with any
-- project alias, and vice versa, so external task ids never resolve ambiguously.
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
