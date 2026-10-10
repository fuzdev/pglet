-- The same order one level out, where the junk follows the whole value.
-- @expect-error: the code-point-zero escape, not the junk after it
-- @expect-message
-- @test
SELECT (E'"\\u0000"\x0c')::jsonb;
