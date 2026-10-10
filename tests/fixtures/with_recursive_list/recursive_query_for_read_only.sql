-- `FOR READ ONLY` locks nothing, so it is no decoration and the recursive query answers.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 FOR READ ONLY) SELECT * FROM r;
