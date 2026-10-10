-- The unknown-constant rule holds for `INTERSECT` and `EXCEPT` as for `UNION`; `01` read
-- as `integer` matches `1`, where a `text` comparison would keep it apart.
-- @test
SELECT count(*) AS n FROM ((SELECT 1 INTERSECT SELECT '01') UNION ALL (SELECT 2 EXCEPT SELECT '02')) s;
