-- The mirror of `default_unknown_literal_runs_its_input_function_at_ddl.sql`, and a
-- **cross-code** pair: a value-dependent failure is *accepted* at DDL and raised per
-- row, so the `CREATE TABLE` here succeeds (a same-code pair would be satisfied by an
-- engine that answered one code to everything). Measured on 18.6: `DEFAULT 1/0` is
-- accepted and `22012` at the first insert.
-- @setup
CREATE TABLE def_div (a INT, b INT DEFAULT 1/0);
-- @expect-error: the division is evaluated per row, not while cooking the default
-- @test
INSERT INTO def_div (a) VALUES (1);
