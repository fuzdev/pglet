-- `ORDER BY … DESC` with `LIMIT` over a series.
-- @test
SELECT g FROM generate_series(1, 10) g ORDER BY g DESC LIMIT 3;
