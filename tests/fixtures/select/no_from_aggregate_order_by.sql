-- A no-FROM aggregate under an `ORDER BY` its ordinal names.
-- @test
SELECT count(*) ORDER BY 1;
