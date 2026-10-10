-- Data-correctness + ON CONFLICT dedup for query_put_fact_refs. The spine's
-- `INSERT … SELECT $src, unnest($targets) ON CONFLICT DO NOTHING` is run twice
-- with overlapping target sets (literal arrays via simple query in @setup); the
-- second run's overlap (`blake3:t2`) is deduped, so the final fact_ref holds the
-- union {t1,t2,t3}. The @test SELECT compares the resulting rows vs real Postgres.
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL, bytes BYTEA);
CREATE TABLE fact_ref (
  source_hash TEXT NOT NULL REFERENCES fact(hash) ON DELETE CASCADE,
  target_hash TEXT NOT NULL,
  PRIMARY KEY (source_hash, target_hash)
);
INSERT INTO fact (hash, size, bytes) VALUES ('blake3:src', 1, '\x00');
INSERT INTO fact_ref (source_hash, target_hash)
       SELECT 'blake3:src'::text, unnest('{blake3:t1,blake3:t2}'::text[])
       ON CONFLICT (source_hash, target_hash) DO NOTHING;
INSERT INTO fact_ref (source_hash, target_hash)
       SELECT 'blake3:src'::text, unnest('{blake3:t2,blake3:t3}'::text[])
       ON CONFLICT (source_hash, target_hash) DO NOTHING;
-- @test
SELECT source_hash, target_hash FROM fact_ref ORDER BY target_hash;
