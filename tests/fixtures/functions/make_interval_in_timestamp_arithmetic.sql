-- A cutoff computed from now-style arithmetic over a named-argument interval.
-- @param $1 int4 3
-- @test
SELECT ('2026-01-02 00:00:00+00'::timestamptz - make_interval(hours => $1))::text AS cutoff;
