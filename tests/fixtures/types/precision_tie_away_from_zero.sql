-- Ties round **away from zero**, not to even — `.0025` is `.003`, which round-half-to-
-- even would make `.002`. `time` is microseconds since midnight and never negative, so
-- the negative side of the split is `interval`'s to show.
-- @test
SELECT '00:00:00.0005'::time(3) AS a,
       '00:00:00.0015'::time(3) AS b,
       '00:00:00.0025'::time(3) AS c,
       '00:00:00.0014'::time(3) AS d,
       '-0.0005 secs'::interval(3) AS e,
       '-0.0015 secs'::interval(3) AS f,
       '-0.0025 secs'::interval(3) AS g,
       '-0.0004 secs'::interval(3) AS h;
