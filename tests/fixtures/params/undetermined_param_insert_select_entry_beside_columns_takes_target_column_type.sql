-- The same over a source with a FROM clause, its other entry a column: `$1` is the
-- `integer` target's, `$2` the comparison's.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
CREATE TABLE up_kv (k TEXT PRIMARY KEY, v JSONB, n INT);
-- @param $1 int4 7
-- @param $2 int4 1
-- @test
INSERT INTO up_kv (k, n) SELECT name, $1 FROM up_t WHERE id = $2 RETURNING k, n;
