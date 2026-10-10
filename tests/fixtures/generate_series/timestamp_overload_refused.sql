-- The timestamp overloads are not modeled; 18.6 answers (pglet-stricter).
-- @expect-error: not implemented: generate_series over timestamps
-- @test
SELECT * FROM generate_series('2024-01-01'::timestamp, '2024-01-03', '1 day');
