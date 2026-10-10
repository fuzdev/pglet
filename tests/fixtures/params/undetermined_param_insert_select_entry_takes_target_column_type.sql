-- A bare `$N` entry of an `INSERT … SELECT` source is coerced to the column it is
-- assigned to — `jsonb` and `integer` here — and never becomes the select list's `text`.
-- @setup
CREATE TABLE up_kv (k TEXT PRIMARY KEY, v JSONB, n INT);
-- @param $1 text z
-- @param $2 jsonb {"a": 1}
-- @param $3 int4 5
-- @test
INSERT INTO up_kv (k, v, n) SELECT $1, $2, $3 RETURNING k, v, n;
