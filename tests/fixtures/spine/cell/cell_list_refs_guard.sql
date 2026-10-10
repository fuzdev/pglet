-- The spine's cell_list `refs` filter (fuz_app's `db/cell_queries.ts`): the optional
-- array-containment clause `($3::text IS NULL OR c.refs @> ARRAY[$3]::text[])` over the partial
-- gin(refs) index, with the param bound NON-NULL. The `IS NULL` disjunct is a guard
-- (false here), so the `@>` probe drives a GIN union; the partial index's
-- `refs IS NOT NULL` is auto-implied by `@>` and `deleted_at IS NULL` is asserted.
-- Row 5 (deleted) must be excluded by the partial index; row 4 (NULL refs) doesn't
-- contain the probe.
-- @profile-lock
-- @setup
CREATE TABLE cell (id int4, refs text[], deleted_at timestamptz);
CREATE INDEX idx_cell_refs ON cell USING gin(refs) WHERE refs IS NOT NULL AND deleted_at IS NULL;
INSERT INTO cell (id, refs, deleted_at) VALUES
  (1, '{a,b}', NULL),
  (2, '{c}',   NULL),
  (3, '{a,c}', NULL),
  (4, NULL,    NULL),
  (5, '{a}',   '2020-01-01T00:00:00Z');
-- @param $1 text a
-- @test
SELECT id FROM cell
WHERE deleted_at IS NULL
  AND ($1::text IS NULL OR refs @> ARRAY[$1]::text[])
ORDER BY id;
