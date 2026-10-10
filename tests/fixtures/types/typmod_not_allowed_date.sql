-- `date` is a temporal type that carries **no** modifier (unlike `time`,
-- `timestamp`, `timestamptz` and `interval`, whose precision it does take), so
-- `date(3)` is `42601` where `time(3)` is accepted.
-- @expect-error: date takes no type modifier
-- @expect-message
-- @test
SELECT '2020-01-01'::date(3) AS never_reached;
