-- `CONCURRENTLY` sits where it does in the named spelling, and the name is still
-- optional behind it, `UNIQUE` included. `@no-txn` because the harness's wrapper would
-- make each of these the `25001` of a `CONCURRENTLY` inside a transaction block.
-- @no-txn
-- @setup
DROP TABLE IF EXISTS ciu20;
CREATE TABLE ciu20 (a INT, b INT);
CREATE INDEX CONCURRENTLY ON ciu20 (a);
CREATE UNIQUE INDEX CONCURRENTLY ON ciu20 (b);
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu20' ORDER BY indexname;
