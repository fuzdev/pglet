-- `t.*` under a column list expands the listed name.
-- @test
SELECT t.*, generate_series.* FROM generate_series(1, 2) AS t(i), generate_series(5, 5) ORDER BY 1;
