-- `x % -1 == 0` for every integer, including `int8` MIN: `checked_rem` alone
-- returns None for `MIN % -1` (the division overflows), so it's special-cased to 0
-- rather than a spurious out-of-range error, matching Postgres.
-- @test
SELECT (-9223372036854775808)::int8 % (-1)::int8 AS a;
