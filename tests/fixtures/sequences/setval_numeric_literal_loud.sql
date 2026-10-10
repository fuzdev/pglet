-- A decimal literal is `numeric`, whose cast to `bigint` is assignment-only — so no
-- overload takes it. Nothing about NULL: the gate reads the argument's static type,
-- and pglet answered 42804 from the value coercion instead.
-- @setup
CREATE SEQUENCE svn_seq;
-- @test
SELECT setval('svn_seq', 5.0);
-- @expect-error: no setval takes a numeric second argument (42883)
