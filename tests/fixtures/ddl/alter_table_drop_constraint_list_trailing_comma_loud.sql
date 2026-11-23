-- A list ending in a comma is a syntax error, before any relation is looked up.
-- @expect-error: syntax error at the end of the list
-- @no-error-fields: both engines send a position, and pglet's parser may stop at a different token
-- @test
ALTER TABLE nosuch_dctc DROP CONSTRAINT x,;
