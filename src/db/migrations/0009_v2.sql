-- v2: drop full-text search, add task dependencies (iss-0071) and status icons (iss-0070),
-- and make `parked` an open status so parked blockers still block.

DROP TABLE IF EXISTS search_fts;
DROP TABLE IF EXISTS search_meta;
DROP TABLE IF EXISTS collections;

CREATE TABLE task_deps (
  task_id    INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  depends_on INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  kind       TEXT NOT NULL DEFAULT 'blocks' CHECK (kind IN ('blocks', 'related')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (task_id, depends_on),
  CHECK (task_id <> depends_on)
);
CREATE INDEX idx_task_deps_depends_on ON task_deps(depends_on);

ALTER TABLE task_statuses ADD COLUMN icon TEXT;

UPDATE task_statuses SET is_closed = 0 WHERE slug = 'parked';

UPDATE task_statuses SET icon = 'backlog'        WHERE slug = 'backlog';
UPDATE task_statuses SET icon = 'queued'   WHERE slug IN ('today', 'cycle');
UPDATE task_statuses SET icon = 'in_progress'      WHERE slug = 'in_progress';
UPDATE task_statuses SET icon = 'in_review'       WHERE slug = 'in_review';
UPDATE task_statuses SET icon = 'done'         WHERE slug = 'done';
