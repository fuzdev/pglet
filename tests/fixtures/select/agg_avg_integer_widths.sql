-- `avg` over each integer width is `numeric`; `sum` widens `smallint`/`integer` to
-- `bigint` and `bigint` to `numeric`.
-- @setup
CREATE TABLE t (a SMALLINT, b INT, c BIGINT);
INSERT INTO t VALUES (3, 3, 3), (4, 4, 4), (6, 6, 6), (NULL, NULL, NULL);
-- @test
SELECT avg(a) AS aa, avg(b) AS ab, avg(c) AS ac, sum(a) AS sa, sum(b) AS sb, sum(c) AS sc FROM t;
