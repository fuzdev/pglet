-- The third argument is `boolean`, and `integer` is not implicitly boolean — so a
-- NULL::int `is_called` is an overload failure, not a strict NULL.
-- @setup
CREATE SEQUENCE svi_seq;
-- @test
SELECT setval('svi_seq', 5, NULL::int);
-- @expect-error: no setval takes an integer third argument (42883)
