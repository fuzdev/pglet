-- The **acceptance** half of `default_value_dependent_error_is_per_row.sql`, and it
-- has to be a success fixture rather than an `@expect-error` one: `@expect-error`
-- compares SQLSTATE, and an engine that raised the division at DDL time would raise
-- the *same* `22012` and pass the error half unchanged. Only a fixture that requires
-- the `CREATE TABLE` to succeed separates "accepted, raised per row" from "raised
-- while cooking".
-- @setup
CREATE TABLE def_div_ok (a INT, b INT DEFAULT 1/0);
-- @test
SELECT column_default IS NOT NULL AS has_default
FROM information_schema.columns WHERE table_name = 'def_div_ok' AND column_name = 'b';
