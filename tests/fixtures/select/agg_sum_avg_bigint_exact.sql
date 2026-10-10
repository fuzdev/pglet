-- `sum(bigint)` is an exact `numeric`, so three `bigint` maxima total
-- `27670116110564327421` rather than `22003`; `avg(bigint)` divides that exact total,
-- so it is `9223372036854775807`, and three `9007199254740993`s average to
-- `9007199254740993.0000` where an `f64` accumulator answers `9007199254740992`.
-- @setup
CREATE TABLE t (g INT, v BIGINT);
INSERT INTO t VALUES (1, 9223372036854775807), (1, 9223372036854775807), (1, 9223372036854775807),
  (2, 9007199254740993), (2, 9007199254740993), (2, 9007199254740993),
  (3, -9223372036854775808), (3, -9223372036854775808), (3, 1);
-- @test
SELECT g, sum(v) AS s, avg(v) AS a FROM t GROUP BY g ORDER BY g;
