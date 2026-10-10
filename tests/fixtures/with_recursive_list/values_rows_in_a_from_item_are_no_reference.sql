-- The rows of a `VALUES` list in a `FROM` item are expressions: `b` is the column, not
-- the CTE named `b`.
-- @setup
CREATE TABLE t (a INT, b INT);
INSERT INTO t VALUES (1, 7);
-- @test
WITH RECURSIVE b AS (SELECT (SELECT max(v) FROM (VALUES (1), (b)) z (v)) AS m FROM t)
SELECT * FROM b;
