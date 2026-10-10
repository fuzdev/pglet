-- Over no rows `avg(integer)` and `sum(bigint)` are NULL, declared `numeric`;
-- `sum(integer)` is a NULL `bigint`.
-- @setup
CREATE TABLE t (a SMALLINT, b INT, c BIGINT);
-- @test
SELECT avg(a) AS aa, avg(b) AS ab, avg(c) AS ac, sum(a) AS sa, sum(b) AS sb, sum(c) AS sc FROM t;
