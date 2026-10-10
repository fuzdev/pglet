-- The same fold as a derived table's column: unified, the column is a certain `double
-- precision`, and `sum` and `avg` over it answer. Declared at the operand subquery's
-- `numeric` over `double precision` values, each was `0A000`, the exact accumulator
-- refusing a value its declaration does not hold.
-- @setup
CREATE TABLE t (k INT, f FLOAT8);
INSERT INTO t VALUES (1, 0.1234567890123456789), (2, 1e300), (3, 2.5);
-- @test
SELECT sum(y) AS s, avg(y) AS a, max(y) AS m, count(y) AS c
FROM (SELECT COALESCE(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > t.k * 2), t.f + 0)
        AS y FROM t WHERE k <> 2) q;
