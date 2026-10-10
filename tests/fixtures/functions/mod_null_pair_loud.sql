-- Two bare NULLs are as ambiguous: 42725.
-- @test
-- @expect-error: function mod(unknown, unknown) is not unique
SELECT mod(NULL, NULL);
