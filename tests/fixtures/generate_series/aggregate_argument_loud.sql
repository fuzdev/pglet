-- An aggregate in a FROM function's argument is 42803.
-- @expect-error: aggregate functions are not allowed in functions in FROM
-- @test
SELECT * FROM generate_series(1, sum(1));
