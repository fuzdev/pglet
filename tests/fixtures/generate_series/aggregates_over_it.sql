-- Aggregates over a series: `sum` of `integer` is `bigint`, `avg` `numeric`.
-- @test
SELECT count(*), sum(g), avg(g), min(g), max(g) FROM generate_series(1, 100) g;
