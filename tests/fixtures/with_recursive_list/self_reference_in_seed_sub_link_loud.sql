-- A sub-link overrides the non-recursive term's own context.
-- @expect-error: self-reference within a sub-link in the seed (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT (SELECT 1 FROM r) UNION ALL SELECT 2)
SELECT * FROM r;
