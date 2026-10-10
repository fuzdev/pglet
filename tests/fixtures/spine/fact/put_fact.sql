-- query_put_fact (fuz_app's `db/fact_queries.ts`): idempotent fact insert returning `hash`
-- when a new row landed. `ON CONFLICT (hash) DO NOTHING` makes a re-put a
-- no-op (no returned row). Here the hash is fresh, so one row returns.
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
-- @param $1 text blake3:newhash
-- @param $2 text https://example.test/blob
-- @param $3 text text/plain
-- @param $4 int8 42
-- @test
INSERT INTO fact (hash, bytes, external_url, content_type, size)
             VALUES ($1, NULL, $2, $3, $4)
             ON CONFLICT (hash) DO NOTHING
             RETURNING hash;
