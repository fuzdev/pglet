-- The name is chosen against the catalog as it stands at the statement: dropping the
-- first unnamed index frees `ciu25_a_idx`, and the next unnamed statement takes it back
-- rather than counting on from the highest digit handed out.
-- @setup
CREATE TABLE ciu25 (a INT);
CREATE INDEX ON ciu25 (a);
CREATE INDEX ON ciu25 (a);
DROP INDEX ciu25_a_idx;
CREATE UNIQUE INDEX ON ciu25 (a);
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu25' ORDER BY indexname;
