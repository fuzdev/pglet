-- A zero step is 22023 when the item runs.
-- @expect-error: step size cannot equal zero
-- @test
SELECT * FROM generate_series(1, 3, 0);
