-- The DELETE leg of `insert/missing_table_precedes_clause_error.sql` — a separate
-- lowering with its own early return, so it needs its own lock. Both clauses would
-- raise `42804` on a table that existed.
-- @expect-error: the missing relation outranks both ill-typed clauses (42P01, not 42804)
-- @test
DELETE FROM no_such_delete_target
  WHERE COALESCE(1, 'x'::TEXT) IS NULL
  RETURNING COALESCE(1, 'x'::TEXT);
