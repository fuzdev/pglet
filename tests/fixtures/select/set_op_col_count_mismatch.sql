-- The two sides of a set operation must have the same number of columns, else a loud
-- 42601 (Postgres: "each UNION query must have the same number of columns").
-- @expect-error: UNION sides have differing column counts
-- @test
SELECT 1 UNION SELECT 1, 2;
