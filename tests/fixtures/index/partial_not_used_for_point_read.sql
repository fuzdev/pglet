-- A PARTIAL index must not serve an unconditional equality point-read. The index
-- only contains predicate-satisfying rows, so probing it for `WHERE path = 'x'`
-- would silently drop the rows that fail the predicate (here, the soft-deleted
-- one). The point-read must return BOTH rows, matching Postgres (which won't use a
-- partial index for a query not implied by its predicate).
-- @setup
CREATE TABLE soft_del (id INT, path TEXT, deleted_at TIMESTAMPTZ);
INSERT INTO soft_del VALUES
  (1, 'docs/a', NULL),
  (2, 'docs/a', '2026-01-01 00:00:00Z');
-- Partial UNIQUE index: only live (non-soft-deleted) rows participate, so the two
-- 'docs/a' rows don't conflict.
CREATE UNIQUE INDEX soft_del_path_live ON soft_del(path) WHERE deleted_at IS NULL;
-- @test
-- Both rows share path='docs/a'; the point read must see the soft-deleted one too.
SELECT id, path, deleted_at IS NULL AS live FROM soft_del WHERE path = 'docs/a' ORDER BY id;
