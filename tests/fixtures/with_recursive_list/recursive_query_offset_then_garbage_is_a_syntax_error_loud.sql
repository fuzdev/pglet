-- The clauses over a recursive query pass their grammar before 18.6 refuses them:
-- a token after the OFFSET is the raw parse's `42601`, not the `0A000` refusal.
-- @expect-error: syntax error (42601)
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 LIMIT 1 OFFSET 1 garbage)
SELECT * FROM r;
