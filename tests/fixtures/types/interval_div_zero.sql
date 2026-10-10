-- `interval / 0` is division by zero; Postgres raises 22012 and pglet must match.
-- @test
-- @expect-error: interval division by zero
SELECT INTERVAL '1 day' / 0;
