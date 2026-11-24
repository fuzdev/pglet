-- A window function with no `FROM`: one row, numbered.
-- @test
SELECT row_number() OVER () AS rn, rank() OVER (ORDER BY 1) AS rk;
