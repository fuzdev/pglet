-- @setup
CREATE TABLE events (id INT, started TIMESTAMPTZ, ended TIMESTAMPTZ);
INSERT INTO events VALUES
  (1, '2020-01-01 00:00:00Z', '2020-01-01 06:00:00Z'),  -- 6 hours
  (2, '2020-01-01 00:00:00Z', '2020-01-03 00:00:00Z');  -- 2 days

-- @test
-- `timestamp - timestamp` yields an interval, compared against `INTERVAL '…'`
-- literals on the Postgres estimate. The spine's session-extend shape:
-- `CASE WHEN expires_at - NOW() < INTERVAL '1 day' THEN … END`. Here the interval
-- is only compared (its projection is covered by the `interval_*` fixtures).
SELECT id,
       ended - started < INTERVAL '1 day' AS under_a_day,
       CASE WHEN ended - started < INTERVAL '12 hours' THEN 'short' ELSE 'long' END AS bucket
FROM events
ORDER BY id ASC;
