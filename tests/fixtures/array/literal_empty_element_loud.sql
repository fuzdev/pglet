-- An array literal's delimiter stands only after an element: two in a row is malformed,
-- for an element type whose own reader would take the empty string too.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{a,,b}'::text[];
