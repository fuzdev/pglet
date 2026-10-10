-- An `ICONST` is an integer that fits `int32`, so a modifier that does not is a
-- **syntax** error on 18.6 rather than a clamp — unlike `timestamp(2147483647)`, which
-- does fit and clamps to 6.
-- @no-error-fields: both engines send a `P`, and pglet's parser reports one token later — it
--   validates the number after consuming it, where 18.6's lexer refuses the `ICONST`
--   at the number itself.
-- @expect-error: a modifier number that does not fit int4
-- @test
SELECT '2020-01-02'::timestamp(99999999999999);
