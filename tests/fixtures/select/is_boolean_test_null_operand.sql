-- A NULL operand, which is the case the family exists for. Measured on 18.6:
-- `NULL IS TRUE` is `f`, `NULL IS UNKNOWN` is `t`, and a *typed* NULL behaves the
-- same as an untyped one.
-- @test
SELECT
  NULL IS TRUE AS a,
  NULL IS NOT TRUE AS b,
  NULL IS UNKNOWN AS c,
  NULL IS NOT UNKNOWN AS d,
  NULL::bool IS NOT FALSE AS e,
  NULL::bool IS FALSE AS f;
