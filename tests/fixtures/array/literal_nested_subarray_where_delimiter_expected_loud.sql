-- A sub-array directly after another, with no delimiter between, is malformed rather than
-- a second dimension.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{{a}{b}}'::text[];
