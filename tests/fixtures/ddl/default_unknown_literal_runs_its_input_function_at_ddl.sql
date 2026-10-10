-- **Which coercion runs at DDL time is a precise rule, not "coercion errors are
-- DDL-time".** Only the target type's *input function on an unknown literal* runs in
-- parse analysis, so `DEFAULT 'x'` into an `INT` is `22P02` at `CREATE TABLE` on both
-- engines. Its mirror — `default_value_dependent_error_is_per_row.sql` — is the half
-- that identifies the rule rather than an engine that refuses every bad default.
-- @expect-error: the integer input function rejects 'x' while cooking the default
-- @test
CREATE TABLE def_in_fn (a INT DEFAULT 'x');
