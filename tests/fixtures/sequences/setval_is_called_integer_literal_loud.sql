-- `setval(s, 5, 1)` is an overload failure where `setval(s, 5, 'true')` is fine: the
-- bare string literal stays `unknown` and coerces to boolean, while `1` is concretely
-- `integer`.
-- @setup
CREATE SEQUENCE svl_seq;
-- @test
SELECT setval('svl_seq', 5, 1);
-- @expect-error: no setval takes an integer third argument (42883)
