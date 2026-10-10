-- An element is quoted whole or not at all: a double quote inside an unquoted element is
-- malformed.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a"b}'::text[];
