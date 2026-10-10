-- The square root of a negative number is undefined (Postgres 2201F, shared with
-- the power-function domain errors), for both the float8 and numeric overloads.
-- @test
-- @expect-error: sqrt of a negative number
SELECT sqrt(-1);
