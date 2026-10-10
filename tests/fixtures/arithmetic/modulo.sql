-- Modulo `%`: integer (`int2/4/8`) and `numeric`, with the remainder taking the
-- DIVIDEND's sign; a mixed int/numeric operand casts the int to numeric, and
-- mixed integer widths promote to the wider. (`x % 0` and a float operand are
-- separate @expect-error fixtures.)
-- @test
SELECT
  7 % 3 AS a,
  (-7) % 3 AS b,
  7 % (-3) AS c,
  10 % 4 AS d,
  10.5 % 3 AS e,
  10 % 2.5 AS f,
  (17::int8) % 5 AS g;
