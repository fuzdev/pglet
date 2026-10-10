-- query_delete_fact (fuz_app's `db/fact_queries.ts`): drop a fact row, returning
-- `(size, external_url)` for disk unlink + byte tallying. Cascades
-- `fact_ref` via the `source_hash` FK.
-- @profile-lock
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
INSERT INTO fact (hash, external_url, size) VALUES
  ('blake3:gone', 'https://example.test/blob', 4096);
-- @param $1 text blake3:gone
-- @test
DELETE FROM fact WHERE hash = $1 RETURNING size, external_url;
