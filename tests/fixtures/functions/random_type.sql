-- random()'s declared type is double precision, and its unaliased column is `random`
-- (read by name from a derived table); the values are masked so the row compares.
-- @test
SELECT random() * 0 AS z, random() < 1 AS lt1, floor(random()) AS fl, s.random * 0 AS named
FROM (SELECT random()) s;
