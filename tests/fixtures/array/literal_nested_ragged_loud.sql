-- A nested literal is walked as 18.6 walks it: sub-arrays of different lengths are
-- malformed.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{{1,2},{3}}'::int4[];
