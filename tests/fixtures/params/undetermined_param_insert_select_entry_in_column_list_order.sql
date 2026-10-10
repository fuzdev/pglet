-- The entry takes the column its position in the **column list** names, not the
-- table's own order.
-- @setup
CREATE TABLE up_kv (k TEXT PRIMARY KEY, v JSONB, n INT);
-- @param $1 int4 5
-- @param $2 text z
-- @test
INSERT INTO up_kv (n, k) SELECT $1, $2 RETURNING k, n;
