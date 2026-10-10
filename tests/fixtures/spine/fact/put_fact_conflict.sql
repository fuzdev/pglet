-- query_put_fact (fuz_app's `db/fact_queries.ts`): re-putting an existing hash is a no-op
-- via `ON CONFLICT (hash) DO NOTHING` — no row is returned, so the caller
-- skips writing `fact_ref`.
-- @setup
CREATE TABLE fact (
  hash TEXT PRIMARY KEY,
  bytes BYTEA,
  external_url TEXT,
  content_type TEXT,
  size BIGINT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  CONSTRAINT fact_storage_present CHECK (bytes IS NOT NULL OR external_url IS NOT NULL)
);
INSERT INTO fact (hash, bytes, size) VALUES ('blake3:existing', '\x00', 1);
-- @param $1 text blake3:existing
-- @param $2 text NULL
-- @param $3 text NULL
-- @param $4 int8 1
-- @test
INSERT INTO fact (hash, bytes, external_url, content_type, size)
             VALUES ($1, '\x00'::bytea, $2, $3, $4)
             ON CONFLICT (hash) DO NOTHING
             RETURNING hash;
