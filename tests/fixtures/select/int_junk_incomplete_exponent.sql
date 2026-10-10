-- An incomplete exponent (`1e` with no digits) is trailing junk, not the number
-- `1` followed by a column `e` (Postgres's `integer_junk`/`realfail` rules).
-- SQLSTATE 42601.
-- @test
-- @expect-error: trailing junk after numeric literal (incomplete exponent)
SELECT 1e;
