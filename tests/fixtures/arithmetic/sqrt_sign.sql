-- `sqrt`/`sign` have only `(float8)`/`(numeric)` overloads, so an INTEGER argument
-- resolves to float8 (not numeric, unlike ceil/floor). `sign` reproduces Postgres's
-- `>`/`<` sign test, so `sign('nan'::float8)` is 0 while `sign('nan'::numeric)` is NaN.
-- @test
SELECT
  sqrt(4) AS a,
  sqrt(4::numeric) AS b,
  sqrt(2.25::numeric) AS c,
  sign(-5) AS d,
  sign(3.7) AS e,
  sign('nan'::float8) AS f,
  sign('nan'::numeric) AS g;
