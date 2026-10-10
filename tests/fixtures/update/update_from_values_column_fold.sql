-- An `UPDATE … FROM` source's `VALUES` column is a typed branch of a fold in `SET` and in
-- `RETURNING`: the fold unifies at the common type and the computed sibling keeps its
-- value, where it was cast to the column's `integer`.
-- @setup
CREATE TABLE t (k INT, f FLOAT8, n NUMERIC);
INSERT INTO t VALUES (1, 1.5, 0), (2, 2.5, 0);
-- @test
UPDATE t SET n = COALESCE(v.x, t.f + 0) FROM (VALUES (2, NULL::int)) v(k, x)
WHERE v.k = t.k RETURNING t.k, n, COALESCE(v.x, t.f + 0) AS c;
