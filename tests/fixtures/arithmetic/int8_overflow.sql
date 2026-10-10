-- Int8 arithmetic overflow errors ("bigint out of range", Postgres 22003).
-- @test
-- @expect-error: bigint out of range
SELECT 9223372036854775807 + 1;
