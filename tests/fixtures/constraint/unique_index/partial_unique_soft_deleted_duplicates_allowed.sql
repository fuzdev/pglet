-- Two rows with the same path but a NON-NULL deleted_at both FAIL the partial
-- index predicate (`WHERE deleted_at IS NULL`), so neither is a uniqueness
-- candidate and both insert (soft-deleted duplicates are allowed).
-- @setup
CREATE TABLE partial_softdel (id INT, path TEXT, deleted_at TIMESTAMPTZ);
CREATE UNIQUE INDEX ux_partial_softdel ON partial_softdel (path) WHERE deleted_at IS NULL;
INSERT INTO partial_softdel VALUES (1, '/b', '2020-01-01T00:00:00Z'), (2, '/b', '2021-01-01T00:00:00Z');

-- @test
SELECT id, path FROM partial_softdel ORDER BY id;
