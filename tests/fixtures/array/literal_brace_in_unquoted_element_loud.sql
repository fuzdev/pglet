-- An opening brace inside an unquoted element is malformed; it is not the element's
-- character.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a{b}'::text[];
