-- Two empty sub-arrays with no delimiter between: an opening brace cannot stand where a
-- delimiter is expected, though neither sub-array holds an element to refuse.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{{}{}}'::int4[];
