-- @test
-- @expect-error: a backslash ending the string is 22P02
SELECT 'ab\'::bytea;
