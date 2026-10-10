-- A window function in a FROM function's argument is 42P20.
-- @expect-error: window functions are not allowed in functions in FROM
-- @test
SELECT * FROM generate_series(1, row_number() OVER ());
