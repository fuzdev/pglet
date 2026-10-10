-- The fuz spine's own shipped schema, its shapes verbatim over a column subset
-- (fuz_app's `db/cell_ddl.ts`), read through statement 4 of the six its snapshot
-- issues — the constraint rows. Three of its shapes once diverged: the enum column's
-- `DEFAULT 'private'`, the partial index's `WHERE moderation = 'pending'`, and
-- `cell_grant_level_check`'s `level IN ('viewer', 'editor')`. Only the last reaches the
-- compared rows; the other two are in the schema, not the output. The two-arm CHECK
-- beside it is the polarity that was already byte-identical.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  visibility  cell_visibility NOT NULL DEFAULT 'private',
  moderation  TEXT,
  root_id     UUID,
  deleted_at  TIMESTAMPTZ
);
CREATE INDEX idx_cell_moderation_pending ON cell(root_id) WHERE moderation = 'pending';
CREATE TABLE cell_grant (
  id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  level    TEXT NOT NULL CHECK (level IN ('viewer', 'editor')),
  actor_id UUID,
  role     TEXT,
  scope_id UUID,
  CHECK ((actor_id IS NOT NULL AND role IS NULL AND scope_id IS NULL)
      OR (actor_id IS NULL AND role IS NOT NULL))
);
-- @param $1 text public
-- @test
SELECT cls.relname::text AS table_name, c.conname::text AS conname,
       c.contype::text AS contype, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c
JOIN pg_namespace n ON n.oid = c.connamespace
JOIN pg_class cls ON cls.oid = c.conrelid
WHERE n.nspname::text = $1 AND c.conrelid != 0 AND c.contype::text <> 'n'
ORDER BY table_name ASC, conname ASC
