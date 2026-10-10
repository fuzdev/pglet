-- Arithmetic leaves an infinity where it is: `± integer`, `± interval` (a timestamp),
-- and `+ time`, even past the range a finite date would leave.
-- @test
SELECT date 'infinity' + 1 AS a, date '-infinity' - 2147483647 AS b, date 'infinity' + 2147483647 AS c,
       date 'infinity' - interval '1 day' AS d, date '-infinity' + interval '10000 years' AS e,
       date 'infinity' + time '24:00' AS f, timestamp 'infinity' + interval '1 mon' AS g,
       timestamptz '-infinity' - interval '1 year' AS h, interval '1 day' + timestamp '-infinity' AS i;
