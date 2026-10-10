-- The control: `round(numeric)` rounds a tie half away from zero.
-- @test
SELECT round(2.5) AS a, round(-2.5) AS b, round(0.5) AS c, round(-0.5) AS d,
       round(2.5, 0) AS e;
