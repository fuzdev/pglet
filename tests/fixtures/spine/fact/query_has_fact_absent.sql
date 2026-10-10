-- query_has_fact (fuz_app's `db/fact_queries.ts`), the false case: the bound hash is
-- not in the table, so `EXISTS(SELECT 1 FROM fact WHERE hash = $1)` is false. Same shape as
-- query_has_fact.sql — proves the negative result, not just the positive.
-- @profile-lock
-- @setup
CREATE TABLE fact (
  hash TEXT PRIMARY KEY,
  size BIGINT NOT NULL
);
INSERT INTO fact (hash, size) VALUES ('blake3:exists', 1), ('blake3:other', 2);
-- @param $1 text blake3:missing
-- @test
SELECT EXISTS(SELECT 1 FROM fact WHERE hash = $1) AS exists;
