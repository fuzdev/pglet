-- Two UNIQUE specifications over the same key columns describe one index, and
-- Postgres merges them in `transformIndexConstraints` rather than erroring ("A strict
-- reading of SQL would suggest raising an error instead, but that strikes me as too
-- anal-retentive. - tgl"). Both spellings are common, and both were 42P07 here once
-- the constraint-index name gate landed.
-- @setup
CREATE TABLE ctru1 (a INT UNIQUE, UNIQUE (a));
CREATE TABLE ctru2 (a INT, UNIQUE (a), UNIQUE (a));
CREATE TABLE ctru3 (a INT PRIMARY KEY UNIQUE);

-- @test
SELECT tablename::text AS tablename, indexname::text AS indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename LIKE 'ctru%' ORDER BY tablename, indexname;
