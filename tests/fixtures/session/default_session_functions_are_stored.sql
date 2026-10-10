-- Every session form is accepted as a column `DEFAULT` — 18.6 accepts all of them,
-- and pglet used to refuse each with `0A000 cannot use column reference in DEFAULT
-- expression`, because the parenless spellings fell through to a column reference.
-- @setup
CREATE TABLE ds_defaults (
  a TEXT DEFAULT current_user,
  b TEXT DEFAULT session_user,
  c TEXT DEFAULT user,
  d TEXT DEFAULT current_role,
  e TEXT DEFAULT current_catalog,
  f TEXT DEFAULT current_schema,
  g TEXT DEFAULT current_schema(),
  h TEXT DEFAULT current_database(),
  i TEXT DEFAULT current_setting('search_path')
);
INSERT INTO ds_defaults DEFAULT VALUES;
-- @test
SELECT a, b, c, d, e, f, g, h, i FROM ds_defaults;
