-- @test
-- `sum`/`avg(DISTINCT …)` over equal `numeric` spellings add one value per set, so the
-- result's scale is the scale of the spelling kept: `1.0` and `2.000` met first here.
SELECT sum(DISTINCT x), avg(DISTINCT x), count(DISTINCT x)
FROM (VALUES (1.0::numeric), (1.00), (2.000), (2)) v(x);
