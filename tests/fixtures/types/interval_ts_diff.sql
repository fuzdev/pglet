-- `timestamp - timestamp → interval`, projected: the raw µs difference is justified
-- (≥24h folds into days, month stays 0), so the projection shows days + time.
-- @test
SELECT '2020-01-03 04:05:06'::timestamp - '2020-01-01 00:00:00'::timestamp AS diff,
       '2020-03-01 00:00:00'::timestamp - '2020-01-01 00:00:00'::timestamp AS long_diff;
