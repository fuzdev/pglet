-- A PARTIAL gin index (the spine's shape): only non-deleted rows are indexed. The
-- query asserts the index predicate (`deleted_at IS NULL`), so pglet is allowed to
-- use the partial GIN; the result must match Postgres.
-- @profile-lock
-- @setup
CREATE TABLE gin_part (id int4, refs text[], deleted_at timestamptz);
CREATE INDEX gin_part_idx ON gin_part USING gin(refs) WHERE deleted_at IS NULL;
INSERT INTO gin_part (id, refs, deleted_at) VALUES
  (1, '{a}', NULL),
  (2, '{a}', '2020-01-01T00:00:00Z'),
  (3, '{a,b}', NULL);
-- @test
SELECT id FROM gin_part WHERE refs @> ARRAY['a'] AND deleted_at IS NULL ORDER BY id;
