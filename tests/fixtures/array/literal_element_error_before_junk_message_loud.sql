-- For an element type whose input error shares the array reader's code, the message says
-- which came first: the element the brace ended was converted.
-- @test
-- @expect-message
-- @expect-error: the element's own 22P02, naming the element
SELECT '{x}1}'::int4[];
