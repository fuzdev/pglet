-- A CHECK is the second of the four persisted-expression sites the general grammar
-- reaches (column DEFAULT, CHECK, index key part, index predicate — the last two under
-- `niladic_arg_index_expr_loud.sql` / `niladic_arg_index_predicate_loud.sql`), and it
-- would have been the worse over-acceptance: a stored expression naming a signature
-- 18.6 has no function for. 42883 on both engines.
-- @test
-- @expect-error: function now(integer) does not exist
CREATE TABLE nb_check (t TIMESTAMPTZ, CHECK (now(1) > now()));
