-- An unnamed `CREATE UNIQUE INDEX` takes the `idx` label too: `key` is the label of a
-- `UNIQUE` **constraint**'s index, and this index is no constraint's. The table carries
-- one of each over the same column, so both labels are read side by side.
-- @setup
CREATE TABLE ciu7 (a INT, b INT UNIQUE);
CREATE UNIQUE INDEX ON ciu7 (b);
CREATE UNIQUE INDEX ON ciu7 (a);
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu7' ORDER BY indexname;
