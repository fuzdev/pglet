-- An `UPDATE … FROM` source's computed column — a derived table's — is a typed branch of
-- a fold in `SET` and in `RETURNING`, so the fold is declared at the common type,
-- `double precision`, where it was declared at the column's `integer`.
-- @setup
CREATE TABLE t (k INT, f FLOAT8, n NUMERIC);
INSERT INTO t VALUES (1, 1.5, 0), (2, 2.5, 0);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (2, NULL);
-- @test
UPDATE t SET n = COALESCE(s.x * 1, t.f + 0) FROM (SELECT k, i + 0 AS x FROM u) s
WHERE s.k = t.k RETURNING t.k, n, COALESCE(s.x * 1, t.f + 0) AS c;
