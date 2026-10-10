-- The grouped derived column read by the outer query's own aggregate, grouped again, and
-- over no rows — the `numeric` declared on both legs where no value says it.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 1), (NULL, NULL), (5, 9007199254740993), (2, 2), (2, 2);
-- @test
SELECT g > 1 AS big, avg(g), sum(h)
FROM (SELECT i AS g, b AS h FROM t GROUP BY i, b) s
GROUP BY g > 1
ORDER BY 1;
