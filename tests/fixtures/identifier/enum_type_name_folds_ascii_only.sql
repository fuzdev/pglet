-- The post-parser ENUM fold is ASCII — the same width as `lexer::Token::name` — so two
-- type names differing only in a **non-ASCII** letter's case are two types, here and on
-- 18.6 (whose `pg_type` holds `cafÉ` and `café`). No quoting appears anywhere in this
-- fixture: `CAFÉ` is unquoted ordinary DDL, and it is the only spelling in it whose ASCII
-- and Unicode folds differ.
-- The two columns are what make this discriminating, and neither is redundant. A Unicode
-- `to_lowercase()` at `executor::ddl::execute_create_type` collapses the two names to one
-- key, so the second `CREATE TYPE` is `42710` and the setup never finishes. A Unicode
-- fold at `executor::type_resolution::resolve_column_type` instead leaves both types
-- registered and binds `u` — declared `CAFÉ`, which the parser hands on as `cafÉ` — to
-- the *other* type, so the `'a'` insert is `invalid input value for enum café`, naming a
-- label the statement just defined. The `::`/`CAST` path is that site's other caller
-- (`SELECT 'a'::CAFÉ` and `CAST('a' AS CAFÉ)` both answer `a` on both engines); this
-- fixture drives the **column type** position because the mis-binding it separates is a
-- persisted one.
-- @setup
CREATE TYPE CAFÉ AS ENUM ('a');
CREATE TYPE café AS ENUM ('b');
CREATE TABLE ct (u CAFÉ, l café);
INSERT INTO ct VALUES ('a', 'b');

-- @test
SELECT u::text AS u, l::text AS l FROM ct;
