-- A LIMIT past Postgres's bigint range fails loud (22003), not silent-unbounded.
-- @expect-error: bigint out of range (22003)
-- @test
SELECT 1 LIMIT 99999999999999999999;
