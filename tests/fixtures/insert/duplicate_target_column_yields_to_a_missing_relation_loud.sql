-- With no target relation there is no column list to resolve — `setTargetTable` runs
-- ahead of every `transform*Clause` on 18.6 and pglet declines to lower a DML statement
-- whose table the catalog lacks — so `42P01` stands and the duplicate is never reached.
-- That is why the gate lives inside the lowering rather than in the parser, where it
-- would have answered `42701` for a table that is not there.
-- @setup
CREATE TABLE dtcm (a INT, b INT);

-- @expect-error: relation "nosuchtable" does not exist (42P01)
-- @no-profile-lock: the statement never reaches a plan, let alone index selection
-- @test
INSERT INTO nosuchtable (a, a) VALUES (1, 2);
