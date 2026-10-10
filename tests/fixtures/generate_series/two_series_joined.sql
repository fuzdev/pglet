-- Two series items under distinct aliases.
-- @test
SELECT g, h FROM generate_series(1, 3) g, generate_series(2, 3) h WHERE g <= h ORDER BY 1, 2;
