-- 18.6 answers NULL for the whole row of an outer join's null-extended side; pglet would
-- answer an object of NULLs there, so it refuses the reference instead (`0A000`).
-- @expect-error: a whole-row reference on the nullable side of an outer join
-- @test
SELECT to_jsonb(b) FROM (SELECT 1 AS a) a LEFT JOIN (SELECT 2 AS x) b ON false;
