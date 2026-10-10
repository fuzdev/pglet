-- The third of the four persisted-expression sites (column DEFAULT, CHECK, index key
-- part, index predicate), and the second half of the pair with
-- `niladic_arg_index_predicate_loud.sql`. Each reaches `build_call` through the general
-- expression grammar and so inherits the gate; without it, the index key would have been
-- persisted naming a signature 18.6 has no function for. 42883 on both engines.
-- @setup
CREATE TABLE nb_ix_expr (a INT, t TIMESTAMPTZ);
-- @test
-- @expect-error: function now(integer) does not exist
CREATE INDEX nb_ix_expr_ix ON nb_ix_expr ((t > now(1)));
