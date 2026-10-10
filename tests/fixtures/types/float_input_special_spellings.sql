-- The controls: the infinity and NaN spellings `float8in` takes, any case, either sign,
-- padded — each answered, not range-refused.
-- @test
SELECT 'Infinity'::real AS a, '-inf'::float8 AS b, 'NaN'::float8 AS c, '+inf'::real AS d,
       'INFINITY'::float8 AS e, '-nan'::float8 AS f, 'nan'::real AS g,
       ' -Infinity '::float8 AS h;
