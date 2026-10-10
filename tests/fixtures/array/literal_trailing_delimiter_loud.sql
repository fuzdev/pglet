-- A closing brace stands where a delimiter could, not after one.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a,}'::text[];
