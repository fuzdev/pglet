-- ...and the array's body trim with an element after it.
-- @expect-error: a form feed before an array element
-- @expect-message
-- @test
SELECT (E'[\x0c1]')::jsonb;
