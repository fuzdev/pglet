-- The spine's orphan-fact correlated NOT EXISTS over a partial gin(refs): a fact is
-- orphaned when no live (non-deleted) cell's refs contains its hash. The inner `@>`
-- probe is correlated on the OUTER `f.hash`, so pglet evaluates the GinScan probe
-- per outer row (the highest-value spine acceleration). The partial index's
-- `refs IS NOT NULL` is auto-implied by the `@>`; `deleted_at IS NULL` is asserted.
-- @profile-lock
-- @setup
CREATE TABLE fact (hash text);
CREATE TABLE cell (id int4, refs text[], deleted_at timestamptz);
CREATE INDEX cell_refs_gin ON cell USING gin(refs) WHERE refs IS NOT NULL AND deleted_at IS NULL;
INSERT INTO fact (hash) VALUES ('h1'), ('h2'), ('h3');
INSERT INTO cell (id, refs, deleted_at) VALUES
  (1, '{h1}', NULL),
  (2, '{h2}', '2020-01-01T00:00:00Z');
-- @test
SELECT hash FROM fact f
WHERE NOT EXISTS (
  SELECT 1 FROM cell c
  WHERE c.refs @> ARRAY[f.hash]::text[] AND c.deleted_at IS NULL
)
ORDER BY hash;
