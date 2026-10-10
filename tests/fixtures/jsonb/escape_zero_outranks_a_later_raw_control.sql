-- Both errors live inside one string token, so their order is their position in it.
-- The escape stands first here.
-- @expect-error: the code-point-zero escape, which stands first
-- @expect-message
-- @test
SELECT (E'"\\u0000a\tb"')::jsonb;
