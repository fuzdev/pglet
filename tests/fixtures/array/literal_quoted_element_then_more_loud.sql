-- A quoted element ends at its closing quote; more of an element after it is malformed.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{"a"b}'::text[];
