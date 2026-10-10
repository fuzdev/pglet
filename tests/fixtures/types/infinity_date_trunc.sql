-- `date_trunc` leaves an infinity as it is, a `date` source answering a `timestamptz`.
-- @test
SELECT date_trunc('day', timestamp 'infinity') AS a, date_trunc('year', timestamptz '-infinity') AS b,
       date_trunc('month', date 'infinity') AS c, date_trunc('week', date '-infinity') AS d,
       date_trunc('microseconds', timestamp '-infinity') AS e;
