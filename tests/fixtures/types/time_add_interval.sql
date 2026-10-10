-- `time ± interval → time`, wrapping into [00:00, 24:00): `'23:00' + 2h` = 01:00,
-- `'01:00' + 3 days` = 01:00 (the interval's day field is ignored, matching
-- Postgres `time_pl_interval`), and the subtraction wraps the other way.
-- Differential vs Postgres.
-- @test
SELECT '23:00:00'::time + INTERVAL '2 hours'    AS wrap_fwd,
       '01:00:00'::time + INTERVAL '3 days'     AS days_ignored,
       '01:00:00'::time - INTERVAL '2 hours'    AS wrap_back,
       '12:00:00'::time + INTERVAL '90 minutes' AS plus_90m,
       -- The 24:00:00 endpoint participates: minus 1h = 23:00, and +2h wraps to 02:00.
       '24:00:00'::time - INTERVAL '1 hour'     AS endpoint_minus,
       '24:00:00'::time + INTERVAL '2 hours'    AS endpoint_wrap;
