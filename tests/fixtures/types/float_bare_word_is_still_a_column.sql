-- The **control** for the commit above: bare `float`, with no `(`, is an ordinary
-- column reference on both engines (`float` is a `ColId`), so the speculative type-name
-- read must still put the cursor back. A commit keyed on the lead word rather than on
-- the `(` would turn this into a type error.
-- @expect-error: a column that does not exist
-- @expect-message
-- @test
SELECT float;
