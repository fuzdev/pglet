-- A bound integer parameter is an integer argument: `avg($1::int)` is `numeric` and
-- `sum($2::bigint)` is `numeric`, over three rows.
-- @setup
CREATE TABLE t (i INT);
INSERT INTO t VALUES (1), (2), (3);
-- @param $1 int4 5
-- @param $2 int8 9223372036854775807
-- @test
SELECT avg($1::int) AS a, sum($2::bigint) AS s, avg($2::bigint) AS ab FROM t;
