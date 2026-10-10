-- `character varying` is the long spelling of the same half, and it reaches the same
-- message: the name in it is `varchartypmodin`'s, not the text a statement wrote.
-- Closing the modifier gate on the spelling instead would name `character varying` here
-- and `character` in the fixture above.
-- @expect-error: a character varying length below 1
-- @expect-message
-- @test
SELECT 'ab'::character varying(0) AS never_reached;
