-- A scalar subquery argument answers on 18.6; refused here (pglet-stricter).
-- @expect-error: generate_series with a column reference or a subquery
-- @test
SELECT * FROM generate_series(1, (SELECT 2));
