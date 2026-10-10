-- A **typed** non-boolean operand is `42804`, from `planner::legality`'s
-- `boolean_context_legality` — the same rulebook the DDL-time static checker reads, so
-- a query and a stored `CHECK` cannot disagree about it.
-- @test
-- @expect-error: argument of IS TRUE must be type boolean
SELECT 1 IS TRUE;
