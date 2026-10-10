-- `1/0` is `integer` and there is no `setval(regclass, bigint, integer)`, so this never
-- reaches evaluation — the mirror of `setval_argument_error_beats_strictness`, whose
-- `setval(s, 1/0, NULL)` *does* type-check and divides by zero.
-- @setup
CREATE SEQUENCE sva_seq;
-- @test
SELECT setval('sva_seq', NULL, 1/0);
-- @expect-error: the overload is resolved before the arguments are evaluated (42883)
