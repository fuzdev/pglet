-- @test
-- `INTERSECT ALL` counts equal values on each side under the interval estimate:
-- `30 days`, `1 mon` and `720 hours` are one group, two on each side.
SELECT x FROM (VALUES (interval '30 days'), (interval '1 mon')) a(x)
INTERSECT ALL SELECT x FROM (VALUES (interval '1 mon'), (interval '720 hours')) b(x);
