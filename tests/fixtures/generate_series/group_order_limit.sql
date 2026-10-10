-- Grouping, ordering, `DISTINCT`, `LIMIT` and `OFFSET` over a series.
-- @test
SELECT g % 3 AS r, count(*), max(g) FROM generate_series(1, 10) g GROUP BY g % 3 ORDER BY r DESC LIMIT 2 OFFSET 1;
