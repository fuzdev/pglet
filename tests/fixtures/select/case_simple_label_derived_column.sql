-- A simple `CASE` over a `VALUES` list's column and over a derived table's computed
-- column types its string labels against the column, as 18.6 does: the label `'1'` is
-- the integer `1`. Each was `42883 operator does not exist: integer = text` while such
-- a column's type was not certain.
-- @setup
CREATE TABLE t (i INT);
INSERT INTO t VALUES (1), (5);
-- @test
SELECT v.x, CASE v.x WHEN '1' THEN 'one' ELSE 'other' END AS c,
  CASE q.y WHEN '2' THEN 'two' ELSE 'other' END AS d
FROM (VALUES (1), (2)) v(x), (SELECT i + 1 AS y FROM t) q ORDER BY 1, 3;
