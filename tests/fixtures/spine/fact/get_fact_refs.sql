-- query_get_fact_refs (fuz_app's `db/fact_queries.ts`): list declared target hashes for a
-- source fact. Order is unspecified in the spine; the fixture ORDER BYs for
-- a deterministic comparison.
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL, bytes BYTEA);
CREATE TABLE fact_ref (
  source_hash TEXT NOT NULL REFERENCES fact(hash) ON DELETE CASCADE,
  target_hash TEXT NOT NULL,
  PRIMARY KEY (source_hash, target_hash)
);
INSERT INTO fact (hash, size, bytes) VALUES ('blake3:src', 1, '\x00'), ('blake3:other', 1, '\x00');
INSERT INTO fact_ref (source_hash, target_hash) VALUES
  ('blake3:src', 'blake3:t1'),
  ('blake3:src', 'blake3:t2'),
  ('blake3:other', 'blake3:t9');
-- @param $1 text blake3:src
-- @test
SELECT target_hash FROM fact_ref WHERE source_hash = $1 ORDER BY target_hash;
