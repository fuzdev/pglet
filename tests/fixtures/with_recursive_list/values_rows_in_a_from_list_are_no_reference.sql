-- A `VALUES` list beside a table in a `FROM` list: its rows' commas begin no item.
-- @setup
CREATE TABLE t (a INT, b INT);
INSERT INTO t VALUES (1, 7);
-- @test
WITH RECURSIVE b AS (SELECT * FROM t, (VALUES (1, 2), (3, b)) v (p, q))
SELECT count(*) FROM b;
