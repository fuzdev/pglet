-- An element and a sub-array at one level are malformed.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a,{b}}'::text[];
