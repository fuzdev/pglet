-- The same body trim with a pair after it: a body trimmed wide drops the character and
-- the object parses, so this separates the body trim from the pair parser's own.
-- @expect-error: a form feed before an object key
-- @expect-message
-- @test
SELECT (E'{\x0c"k": 1}')::jsonb;
