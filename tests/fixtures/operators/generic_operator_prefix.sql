-- A prefix operator — `gram.y`'s `qual_Op a_expr` — is read as one, and refused for
-- the operator it names rather than for the spelling.
-- @test
-- @expect-error: operator does not exist: ~ text
-- @expect-message
SELECT ~ 'a'::text;
