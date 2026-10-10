-- `ARRAY[…]` resolves the common type of ALL its elements (the same
-- `select_common_type` COALESCE uses) and coerces each one to it. Both of pglet's type
-- walkers used to read the *first* element instead, so `ARRAY[2::int4, 1::int8]`
-- declared `int4[]` over a value whose eval-time common type was `int8[]`.
-- Deliberately NOT cast to ::text — the cast would replace the declared array type and
-- hide the very divergence this locks.
-- @test
SELECT ARRAY[2::INT4, 1::INT8] AS int_widens,
       ARRAY[1, 2.5] AS numeric_widens,
       ARRAY[1, NULL, 2.5] AS numeric_widens_with_null,
       ARRAY[1::INT4, 2::REAL] AS real_wins,
       ARRAY[3::INT8, 4::INT2] AS bigint_wins;
