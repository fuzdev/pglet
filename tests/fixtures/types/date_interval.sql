-- `date ± interval → timestamp`: a bare `date` carries no time zone, so Postgres
-- yields `timestamp` (without tz) at UTC midnight then the 3-pass calendar shift —
-- never `timestamptz`. `interval + date` is commutative. These day/time intervals
-- are byte-stable in the differential harness.
-- @test
SELECT '2024-06-15'::date + INTERVAL '3 days'            AS plus_days,
       '2024-06-15'::date + INTERVAL '5 hours 30 minutes' AS plus_time,
       INTERVAL '2 days' + '2024-06-15'::date            AS commutative,
       '2024-06-15'::date - INTERVAL '1 day'             AS minus_day;
