-- A `WITH` query over a grouped query passes its grouped column through, so `avg` over it
-- is 18.6's exact `numeric` on both legs.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 1), (NULL, NULL), (5, 9007199254740993), (2, 2), (2, 2);
-- @test
WITH c AS (SELECT i AS g, b AS h FROM t GROUP BY i, b) SELECT avg(g), sum(h) FROM c;
