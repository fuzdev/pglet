-- With no target relation the clauses are not lowered at all and the executor's `42P01`
-- stands — the rule `lowers_clauses` states, which is why this gate sits inside the
-- lowering rather than in the parser.
-- @setup
CREATE TABLE dstm (a INT, b INT);

-- @expect-error: relation "nosuchtable" does not exist (42P01)
-- @no-profile-lock: the statement never reaches a plan, let alone index selection
-- @test
UPDATE nosuchtable SET a = 1, a = 2;
