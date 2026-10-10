-- One ACTIVE row + one SOFT-DELETED row with the same path coexist: only the
-- active row is in the partial index (`WHERE deleted_at IS NULL`), so the
-- soft-deleted row never conflicts with it.
-- @setup
CREATE TABLE partial_mixed (id INT, path TEXT, deleted_at TIMESTAMPTZ);
CREATE UNIQUE INDEX ux_partial_mixed ON partial_mixed (path) WHERE deleted_at IS NULL;
INSERT INTO partial_mixed VALUES (1, '/c', NULL), (2, '/c', '2020-01-01T00:00:00Z');

-- @test
SELECT id, path FROM partial_mixed ORDER BY id;
