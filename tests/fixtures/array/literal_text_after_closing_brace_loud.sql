-- The first closing brace at the outer level ends the array; an element's characters
-- after it are junk, not part of the element.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a}b}'::bytea[];
