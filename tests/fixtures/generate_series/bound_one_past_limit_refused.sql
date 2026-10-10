-- One row past the bound is refused, before a row is built; 18.6 answers (pglet-stricter).
-- @expect-error: generate_series would return 1000001 rows
-- @test
SELECT count(*) FROM generate_series(1, 1000001) g;
