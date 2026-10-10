-- `DISTINCT` on a function that resolves is 42809 — ahead of the literal's own 22P02.
-- @expect-error: DISTINCT specified, but generate_series is not an aggregate function
-- @test
SELECT * FROM generate_series(DISTINCT 1, 'x');
