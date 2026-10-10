-- The target column types the entry once the source is analysed, ahead of `RETURNING`:
-- the null test there finds `$2` an `integer`.
-- @setup
CREATE TABLE up_kv (k TEXT PRIMARY KEY, v JSONB, n INT);
-- @param $1 text z
-- @param $2 int4 5
-- @test
INSERT INTO up_kv (k, n) SELECT $1, $2 RETURNING k, n, $2 IS NULL AS z;
