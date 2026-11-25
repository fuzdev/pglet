-- Positional arguments fill years, months, weeks, days, hours, mins and secs in order; a
-- `smallint` widens, a bare string literal reads as the position's type, and a NULL
-- argument answers NULL.
-- @test
SELECT make_interval(1, 2, 3, 4, 5, 6, 7.5) AS all7, make_interval(0, 0, 0, 0, 32767::int2) AS small,
       make_interval(0, 0, 0, 0, '2') AS lit, make_interval(NULL::int) AS nul,
       make_interval(mins => 2147483647) AS big_mins;
