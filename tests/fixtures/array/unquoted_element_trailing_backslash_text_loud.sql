-- A backslash that ends an array literal's body is not kept as a backslash.
-- @test
-- @expect-error: a backslash ending the body of a text[] literal is 22P02
SELECT '{a\}'::text[];
