-- An **index** holds the name: a declared one took `ciu12_a_idx1` first, so the three
-- unnamed indexes are `ciu12_a_idx`, `ciu12_a_idx2` and `ciu12_a_idx3` — the digit skips
-- the taken one and each statement sees the name the one before it minted.
-- @setup
CREATE TABLE ciu12 (a INT, b INT);
CREATE INDEX ciu12_a_idx1 ON ciu12 (b);
CREATE INDEX ON ciu12 (a);
CREATE INDEX ON ciu12 (a);
CREATE INDEX ON ciu12 (a);
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu12' ORDER BY indexname;
