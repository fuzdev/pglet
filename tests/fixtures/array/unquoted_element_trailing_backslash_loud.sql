-- A backslash that ends an array literal's body has no character to take.
-- @test
-- @expect-error: a backslash ending the body of an int4[] literal is 22P02
SELECT '{1\}'::int4[];
