-- The output column type is the common type across all three sides: integer, bigint,
-- and numeric unify to numeric (Postgres `select_common_type`). The fractional 3.5
-- makes the numeric type observable — an integer output column could not carry it.
-- @test
SELECT 1 UNION SELECT 2::int8 UNION SELECT 3.5 ORDER BY 1;
