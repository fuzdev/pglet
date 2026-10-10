-- …and ahead of `ON CONFLICT`.
-- @setup
CREATE TABLE up_kv (k TEXT PRIMARY KEY, v JSONB, n INT);
INSERT INTO up_kv VALUES ('z', NULL, 1);
-- @param $1 text z
-- @param $2 int4 5
-- @test
INSERT INTO up_kv (k, n) SELECT $1, $2 ON CONFLICT (k) DO UPDATE SET n = excluded.n WHERE $2 IS NOT NULL RETURNING k, n;
