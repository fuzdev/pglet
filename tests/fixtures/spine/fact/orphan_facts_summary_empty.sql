-- The orphan summary over no orphan facts: `SUM(size)` over a `BIGINT` is a NULL
-- `numeric`, so `COALESCE` answers its `0` — unified to `numeric` — cast back to `bigint`.
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE cell (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), refs TEXT[] NOT NULL DEFAULT '{}', deleted_at TIMESTAMPTZ);
INSERT INTO fact (hash, size) VALUES ('blake3:live', 20);
INSERT INTO cell (refs, deleted_at) VALUES (ARRAY['blake3:live']::text[], NULL);
-- @param $1 timestamptz NULL
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0)::bigint AS total
FROM fact f
WHERE NOT EXISTS (SELECT 1 FROM cell c WHERE c.refs @> ARRAY[f.hash]::text[] AND c.deleted_at IS NULL)
  AND ($1::timestamptz IS NULL OR f.created_at < $1::timestamptz);
