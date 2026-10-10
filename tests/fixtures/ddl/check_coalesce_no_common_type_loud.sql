-- A **stored** expression with no common type must be rejected at DDL time, where
-- Postgres rejects it (`42804`). Recording it instead left a CHECK constraint that
-- could never mean anything — over-permissiveness rather than the accept-then-brick
-- shapes nearby, since the writes then succeed.
-- This is the construct-level half of the rulebook joining the operand-level half in
-- the same single bottom-up walk that already guards stored expressions.
-- @test
-- @expect-error: CHECK branches have no common type (42804)
CREATE TABLE ckc (a INT4, c TEXT, CHECK (COALESCE(a, c) IS NOT NULL));
