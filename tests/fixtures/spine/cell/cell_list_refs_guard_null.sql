-- The same cell_list `refs` clause with the param bound NULL: the `$3 IS NULL` guard
-- is true, so the OR is unconditionally satisfied and the filter degenerates to
-- `deleted_at IS NULL` over every row. The GinScan returns all rows (guard fires →
-- full scan) and the recheck Filter passes them — proving the optional filter is a
-- no-op when its param is absent, including for the NULL-refs row (4) the partial
-- index doesn't contain.
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
-- @param $1 text NULL
-- @test
SELECT id FROM cell
WHERE deleted_at IS NULL
  AND ($1::text IS NULL OR refs @> ARRAY[$1]::text[])
ORDER BY id;
