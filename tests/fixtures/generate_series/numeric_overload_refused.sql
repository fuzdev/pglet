-- The `numeric` overload is not modeled; 18.6 answers (pglet-stricter).
-- @expect-error: not implemented: generate_series over numeric
-- @test
SELECT * FROM generate_series(1, 3.0);
