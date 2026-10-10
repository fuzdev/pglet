-- `timestamptz ± interval → timestamptz`, the spine's `expires_at = NOW() +
-- INTERVAL '1 hour'` offer/grant shape. Restricted to hour/minute units, which
-- are exact regardless of the session time zone/DST — pglet stores timestamptz as
-- UTC with no session zone, so a calendar-day interval on a timestamptz could
-- diverge from Postgres across a DST boundary (out of scope; use plain timestamp
-- for day intervals). The wire value is microseconds-since-epoch UTC either way.
-- @setup
CREATE TABLE grants (id INT, expires_at TIMESTAMPTZ);
INSERT INTO grants VALUES
  (1, '2020-01-01 12:00:00+00'),
  (2, '2020-01-01 00:30:00+00');

-- @test
SELECT id,
       expires_at + INTERVAL '1 hour' AS plus_1h,
       expires_at - INTERVAL '90 minutes' AS minus_90m
FROM grants
ORDER BY id ASC;
