-- What `pg_get_expr` puts in `column_default` for each stored form, which is the
-- one thing that makes them nine `BuiltinFn` variants rather than three: the six
-- parenless keywords render as their bare upper-case word, `current_schema()`
-- renders **quoted** (an unquoted `current_schema()` would re-lex as the keyword
-- and restore as the other variant), and `current_database()` / `version()` render
-- as ordinary calls.
--
-- `current_setting('search_path')` is deliberately absent: 18.6 renders the
-- implicit unknown→text cast (`current_setting('search_path'::text)`) that pglet
-- omits for *every* text-argument function (`DEFAULT upper('a')` measures the same
-- split), so it belongs to that pre-existing class and not to this fixture.
-- @no-declared-types: `information_schema.columns.column_default` is Postgres's
-- `character varying` (OID 1043) against pglet's synthesized `text` (25) — the same
-- catalog-view type gap the other `information_schema` fixtures waive, and outside
-- this fixture's subject, which is the rendered text.
-- @setup
CREATE TABLE dr_render (
  a TEXT DEFAULT current_user,
  b TEXT DEFAULT session_user,
  c TEXT DEFAULT user,
  d TEXT DEFAULT current_role,
  e TEXT DEFAULT current_catalog,
  f TEXT DEFAULT current_schema,
  g TEXT DEFAULT current_schema(),
  h TEXT DEFAULT current_database(),
  i TEXT DEFAULT version()
);
-- @test
SELECT column_name, column_default FROM information_schema.columns
WHERE table_name = 'dr_render' ORDER BY ordinal_position;
