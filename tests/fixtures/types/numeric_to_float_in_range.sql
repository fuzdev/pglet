-- The controls: in-range numerics convert, `NaN` included.
-- @test
SELECT 1.5::numeric::float8 AS a, 0::numeric::real AS b, 'NaN'::numeric::float8 AS c,
       'NaN'::numeric::real AS d, 1e-40::numeric::real AS e;
