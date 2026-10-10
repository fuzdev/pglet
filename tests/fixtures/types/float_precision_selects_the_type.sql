-- `FLOAT(n)`'s modifier **selects** the type instead of narrowing one: 18.6's
-- `opt_float` reduces to `float4` for `n` in 1–24 and to `float8` for 25–53, in the
-- grammar. So the value each cast target keeps is the *selected* type's precision, and
-- the rounding here is `real`'s and not a modifier's — `0.1234567890123456` is
-- `0.12345679` through 24 and whole from 25 up.
--
-- The `float(25)` and `float(53)` targets are the **controls** on the other side of the
-- split, and bare `float` the control for no modifier at all.
-- @test
SELECT 0.1234567890123456::float(1) AS a,
       0.1234567890123456::float(24) AS b,
       0.1234567890123456::float(25) AS c,
       0.1234567890123456::float(53) AS d,
       0.1234567890123456::float AS e,
       CAST(0.1234567890123456 AS float(1)) AS f,
       FLOAT(1) '0.1234567890123456' AS g,
       '{0.1234567890123456}'::float(1)[] AS h;
