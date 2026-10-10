-- `INTERSECT` binds tighter than `UNION`, so the recursive term is `… FROM r INTERSECT
-- SELECT 2`; a plain `INTERSECT` closes no context, and the term is recursive.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 INTERSECT SELECT 2) SELECT * FROM r;
