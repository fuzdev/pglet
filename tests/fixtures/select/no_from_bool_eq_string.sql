-- A no-FROM comparison of a bool literal against a bare string literal resolves the
-- unknown string to bool (Postgres's unknown-literal rule), so `true = 'yes'` is
-- `t`. Previously the no-FROM path skipped operand coercion and returned `f`.
-- @test
SELECT true = 'yes' AS eq;
