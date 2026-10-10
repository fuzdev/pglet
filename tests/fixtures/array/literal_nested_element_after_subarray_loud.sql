-- A sub-array, then an element at the outer level: every element sits at one depth.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{{a},b}'::text[];
