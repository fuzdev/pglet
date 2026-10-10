-- query_orphan_facts_list (fuz_app's `db/fact_queries.ts`): correlated NOT EXISTS —
-- a fact is orphan iff no live cell references its hash. Outer f.hash is unambiguous
-- (not a cell column).
-- @setup
CREATE TABLE fact (hash TEXT PRIMARY KEY, size BIGINT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE cell (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), refs TEXT[] NOT NULL DEFAULT '{}', deleted_at TIMESTAMPTZ);
INSERT INTO fact (hash, size) VALUES ('blake3:orphan', 10), ('blake3:live', 20), ('blake3:dead', 30);
INSERT INTO cell (refs, deleted_at) VALUES (ARRAY['blake3:live']::text[], NULL), (ARRAY['blake3:dead']::text[], now());
-- @param $1 text NULL
-- @test
SELECT hash, size FROM fact f
WHERE NOT EXISTS (SELECT 1 FROM cell c WHERE c.refs @> ARRAY[f.hash]::text[] AND c.deleted_at IS NULL)
  AND ($1::text::timestamptz IS NULL OR f.created_at < $1::text::timestamptz)
ORDER BY hash;
