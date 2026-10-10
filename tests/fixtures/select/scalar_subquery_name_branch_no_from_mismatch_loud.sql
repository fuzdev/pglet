-- The no-FROM clause is the one whose gate could be handed no catalog of its own: every
-- other clause already carries a schema for its own scope, so this is the only place a
-- bare scalar subquery could defer the gate forever. That is a property of the plumbing,
-- not of a state some build held — the gate's subquery reader and the schema this clause
-- hands it are one change, so no commit ever refused here and deferred there.
-- @test
SELECT COALESCE((SELECT current_user), 1);
-- @expect-error: COALESCE types name and integer cannot be matched
