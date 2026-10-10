-- `sum` over a derived table's computed `bigint` column is `numeric`, so a total past the
-- `bigint` range answers.
-- @setup
CREATE TABLE t (b BIGINT);
INSERT INTO t VALUES (9223372036854775806), (9223372036854775806), (1);
-- @test
SELECT sum(x) AS s, avg(x) AS a FROM (SELECT b + 1 AS x FROM t) q;
