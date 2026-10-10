-- A quoted element followed by more of an element is refused at its closing quote, before
-- the quoted text is converted.
-- @test
-- @expect-error: the array reader's 22P02, not the element's 22007
SELECT '{"x"y}'::date[];
