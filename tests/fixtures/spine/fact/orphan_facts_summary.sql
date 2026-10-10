-- query_orphan_facts_list's summary (fuz_app's `db/fact_queries.ts`) over the spine's own
-- `size BIGINT`: `SUM(bigint)` is `numeric`, `COALESCE` unifies its `0` to `numeric`, and
-- the `::bigint` casts the total back. The declared types are compared on both legs.
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE cell (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), refs TEXT[] NOT NULL DEFAULT '{}', deleted_at TIMESTAMPTZ);
INSERT INTO fact (hash, size) VALUES ('blake3:orphan', 10), ('blake3:big', 9223372036854775000), ('blake3:live', 20), ('blake3:dead', 30);
INSERT INTO cell (refs, deleted_at) VALUES (ARRAY['blake3:live']::text[], NULL), (ARRAY['blake3:dead']::text[], now());
-- @param $1 timestamptz NULL
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0)::bigint AS total
FROM fact f
WHERE NOT EXISTS (SELECT 1 FROM cell c WHERE c.refs @> ARRAY[f.hash]::text[] AND c.deleted_at IS NULL)
  AND ($1::timestamptz IS NULL OR f.created_at < $1::timestamptz);
