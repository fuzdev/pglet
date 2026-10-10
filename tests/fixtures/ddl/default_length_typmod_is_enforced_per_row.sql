-- A `varchar(n)` length is a **typmod**, applied after the type's input function, so
-- 18.6 accepts `DEFAULT 'abcdef'` on a `VARCHAR(3)` at `CREATE TABLE` and raises
-- `22001` at the first row. The pair with
-- `default_unknown_literal_runs_its_input_function_at_ddl.sql` is what separates "the
-- input function" from "any coercion error": both are string literals into a column
-- that cannot hold them, and only one is a DDL-time error.
-- @setup
CREATE TABLE def_len (a INT, b VARCHAR(3) DEFAULT 'abcdef');
-- @expect-error: the length typmod is enforced on the written row, not while cooking
-- @test
INSERT INTO def_len (a) VALUES (1);
