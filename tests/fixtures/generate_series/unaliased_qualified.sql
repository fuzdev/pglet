-- Unaliased, the relation is `generate_series` and so is the column.
-- @test
SELECT generate_series.generate_series FROM generate_series(1, 2);
