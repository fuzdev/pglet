-- `round(double precision)` is `dround`, C's `rint`: a tie rounds to the even integer.
-- @test
SELECT round(2.5::float8) AS a, round(-2.5::float8) AS b, round(0.5::float8) AS c,
       round(-0.5::float8) AS d, round(1.5::float8) AS e, round(3.5::real) AS f;
