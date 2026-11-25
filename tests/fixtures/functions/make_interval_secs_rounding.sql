-- `secs` reaches microseconds rounded half to even, as `rint` rounds.
-- @test
SELECT make_interval(secs => 1.0000005) AS a, make_interval(secs => 0.0000005) AS b,
       make_interval(secs => 0.0000015) AS c, make_interval(secs => 2::bigint) AS d;
