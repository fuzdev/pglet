-- The fourth persisted-expression site: a partial index's predicate, which is a
-- different parse position from the key part its sibling
-- `niladic_arg_index_expr_loud.sql` covers and is validated by a different executor
-- call. Both take the refusal from the one `build_call` above them. 42883 on both
-- engines.
-- @setup
CREATE TABLE nb_ix_pred (a INT, t TIMESTAMPTZ);
-- @test
-- @expect-error: function now(integer) does not exist
CREATE INDEX nb_ix_pred_ix ON nb_ix_pred (a) WHERE t > now(1);
