-- A select-list `generate_series` is 18.6's `ProjectSet`; not modeled (pglet-stricter).
-- @expect-error: function not supported: generate_series
-- @test
SELECT generate_series(1, 3);
