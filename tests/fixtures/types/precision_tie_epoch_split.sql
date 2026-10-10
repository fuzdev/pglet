-- **The rounding sign is the Postgres epoch's, not the wall clock's.** 18.6's
-- `AdjustTimestampForTypmod` adds half a scale and truncates toward zero over
-- microseconds since **2000-01-01**, so an instant before that epoch is negative and
-- its tie rounds *down* in wall-clock terms: `1999-12-31 23:59:59.9995` under `(3)` is
-- `…59.999`, where a naive half-up gives `2000-01-01 00:00:00`. The 1969 row is the
-- same trap read against the Unix epoch, and the 2000 row is the positive control one
-- half-millisecond the other side of the split.
-- @test
SELECT '1999-12-31 23:59:59.9995'::timestamp(3) AS before_epoch,
       '2000-01-01 00:00:00.0005'::timestamp(3) AS after_epoch,
       '1969-07-20 20:17:40.0005'::timestamp(3) AS before_unix_epoch,
       '1999-12-31 23:59:59.9995'::timestamptz(3) AS before_epoch_tz;
