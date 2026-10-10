-- query_put_fact_refs (fuz_app's `db/fact_queries.ts`): record a source fact's
-- target hashes.
-- `INSERT INTO fact_ref (source_hash, target_hash) SELECT $1, unnest($2) ON
-- CONFLICT DO NOTHING` — `$1` (one source hash) is broadcast across the
-- `unnest($2)` row-expansion (the target hashes). This exercises the **bound**
-- extended-protocol path end-to-end: `INSERT … SELECT`, projection-unnest, and
-- the array parameter `$2`. The statement returns no rows (no RETURNING), so the
-- compared output is the empty result + command tag; correctness of the inserted
-- rows is verified by `put_fact_refs_result.sql`.
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL, bytes BYTEA);
CREATE TABLE fact_ref (
  source_hash TEXT NOT NULL REFERENCES fact(hash) ON DELETE CASCADE,
  target_hash TEXT NOT NULL,
  PRIMARY KEY (source_hash, target_hash)
);
INSERT INTO fact (hash, size, bytes) VALUES ('blake3:src', 1, '\x00');
-- @param $1 text blake3:src
-- @param $2 text[] {blake3:t1,blake3:t2}
-- @test
INSERT INTO fact_ref (source_hash, target_hash)
             SELECT $1::text, unnest($2::text[])
             ON CONFLICT (source_hash, target_hash) DO NOTHING;
