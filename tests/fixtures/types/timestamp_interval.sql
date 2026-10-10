-- `timestamp ± interval → timestamp`. These are day/hour intervals, and plain
-- `timestamp` (no time zone) arithmetic treats a day as exactly 24h regardless of
-- the session time zone, so they are byte-stable in the differential harness.
-- `interval + timestamp` is commutative. The spine's offer/grant setup uses
-- `NOW() ± INTERVAL '…'`. (Month-shift calendar arithmetic: `timestamp_interval_month`.)
-- @setup
CREATE TABLE appts (id INT, ts TIMESTAMP);
INSERT INTO appts VALUES
  (1, '2020-01-01 12:00:00'),
  (2, '2020-06-15 23:30:00');

-- @test
SELECT id,
       ts + INTERVAL '3 hours' AS plus_3h,
       ts + INTERVAL '2 days' AS plus_2d,
       ts - INTERVAL '1 day' AS minus_1d,
       INTERVAL '90 minutes' + ts AS commutative_plus
FROM appts
ORDER BY id ASC;
