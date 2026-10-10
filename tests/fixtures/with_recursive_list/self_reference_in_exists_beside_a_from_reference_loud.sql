-- A sub-link self-reference beside the term's one `FROM` reference: 18.6's walk meets
-- the sub-link and reports it before any count.
-- @expect-error: self-reference within a sub-link (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 AND EXISTS (SELECT 1 FROM r))
SELECT * FROM r;
