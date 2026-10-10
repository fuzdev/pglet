-- The function position through a scalar subquery: `abs((SELECT conrelid …))` answered
-- 16384 where 18.6 raises `42883 function abs(oid) does not exist`. The aggregate
-- spelling of this same shape is `agg_expr_oid_math_function_through_an_aggregate_is_loud.sql`.
-- @expect-error: 42883 — a math function over an `oid` produced by a scalar subquery
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT abs((SELECT conrelid FROM pg_constraint LIMIT 1))
