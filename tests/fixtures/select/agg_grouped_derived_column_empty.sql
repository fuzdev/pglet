-- Over no rows a grouped derived column's `sum(bigint)` and `avg` are NULL, declared
-- `numeric` on both legs, and a `COALESCE` default beside them is unified to `numeric`.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
-- @test
SELECT sum(h), avg(g), COALESCE(avg(g), 7), COALESCE(sum(h), 7)
FROM (SELECT i AS g, b AS h FROM t GROUP BY i, b) s;
