-- query_has_fact (fuz_app's `db/fact_queries.ts`): the spine's existence probe, verbatim —
-- `SELECT EXISTS(SELECT 1 FROM fact WHERE hash = $1) AS exists`. Non-correlated
-- scalar EXISTS with a no-FROM outer SELECT (routes through the VALUES path),
-- the inner `hash = $1` typed from the `fact.hash` (TEXT) column. Here $1 names a
-- seeded hash, so EXISTS is true.
-- @profile-lock
-- @setup
CREATE TABLE fact (
  hash TEXT PRIMARY KEY,
  size BIGINT NOT NULL
);
INSERT INTO fact (hash, size) VALUES ('blake3:exists', 1), ('blake3:other', 2);
-- @param $1 text blake3:exists
-- @test
SELECT EXISTS(SELECT 1 FROM fact WHERE hash = $1) AS exists;
