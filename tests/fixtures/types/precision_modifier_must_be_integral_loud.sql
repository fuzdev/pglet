-- A non-integral modifier is the same `Iconst` refusal.
-- @no-error-fields: both engines send a `P` and disagree about which token is wrong: pglet
--   validates the number after consuming it, 18.6's lexer refuses the `ICONST` at the
--   number itself.
-- @expect-error: a fractional modifier number
-- @test
SELECT '2020-01-02'::timestamp(3.5);
