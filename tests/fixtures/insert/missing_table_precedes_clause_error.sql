-- The target relation is resolved **before** any clause. `transformInsertStmt` calls
-- `setTargetTable` first, so a statement naming a relation that does not exist is
-- `42P01` no matter what else is wrong with it.
--
-- An **order lock, not a regression lock**: pglet raises `42P01` at execution, so
-- the rule holds only while `lowers_clauses` keeps a plan-time gate from firing
-- first on an ill-typed clause. This fixture fails the moment one does; it records
-- no past pglet bug.
--
-- Both ill-typed clauses here would raise `42804` on a table that existed, so the
-- relation error is identifiable: `42804` means a clause was lowered against a
-- relation the planner could not see.
-- @expect-error: the missing relation outranks both ill-typed clauses (42P01, not 42804)
-- @test
INSERT INTO no_such_insert_target VALUES (COALESCE(1, 'x'::TEXT))
  ON CONFLICT (id) DO UPDATE SET tx = COALESCE(1, 'x'::TEXT)
  RETURNING COALESCE(1, 'x'::TEXT);
