-- `time(p)` and `interval(p)` round the same way. An interval rounds **only** its
-- micros field — months and days are a *range* spec's business, and a bare
-- `interval(p)` is `INTERVAL_FULL_RANGE`, so `'1 day 1.5 secs'::interval(0)` keeps its
-- day and rounds the seconds.
-- @test
SELECT '03:04:05.6789'::time(3) AS a,
       CAST('03:04:05.6789' AS time(3)) AS b,
       '1.23456 secs'::interval(3) AS c,
       '1.23456 secs'::interval(0) AS d,
       '1 day 1.5 secs'::interval(0) AS e,
       '14 mons 3 days 1.23456 secs'::interval(3) AS f;
