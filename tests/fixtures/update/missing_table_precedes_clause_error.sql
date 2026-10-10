-- The UPDATE leg of `insert/missing_table_precedes_clause_error.sql`:
-- `transformUpdateStmt` resolves the target relation before the `WHERE`, the
-- `RETURNING`, or the `SET` list, so a missing one is `42P01` whatever the clauses say.
-- All three clauses here would raise `42804` on a table that existed.
-- @expect-error: the missing relation outranks all three ill-typed clauses (42P01, not 42804)
-- @test
UPDATE no_such_update_target SET tx = COALESCE(1, 'x'::TEXT)
  WHERE COALESCE(1, 'x'::TEXT) IS NULL
  RETURNING COALESCE(1, 'x'::TEXT);
