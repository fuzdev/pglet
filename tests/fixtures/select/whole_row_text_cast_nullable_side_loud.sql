-- 18.6's whole row of an outer join's null-extended side is NULL; pglet refuses the
-- reference there (`0A000`).
-- @expect-error: a whole-row reference on the nullable side of an outer join
-- @test
SELECT b::text FROM (SELECT 1 AS a) a LEFT JOIN (SELECT 2 AS x) b ON false;
