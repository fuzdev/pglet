-- `information_schema.columns.column_default` reads the same renderer, so a column
-- default's `EXTRACT` field takes `quote_identifier` too: an uppercase or mixed-case
-- string constant and a quoted keyword come back as quoted identifiers, a bare field word
-- and a lowercase string constant bare. `extract_field_quoting_constraintdef.sql` names
-- the rule's members.
-- @no-declared-types: both columns this test selects are **string** ones, and every
-- string column of an `information_schema` view is a domain on 18.6 — `sql_identifier`
-- (over `name`, 19) or `character_data` (over `varchar`, 1043) — where pglet declares the
-- base `text` (25). That divergence is the view layer's, not this fixture's subject,
-- which is the rendered text in the cell.
-- @setup
CREATE TABLE xd (
  a NUMERIC DEFAULT EXTRACT('YEAR' FROM now()),
  b NUMERIC DEFAULT EXTRACT(YEAR FROM now()),
  c NUMERIC DEFAULT EXTRACT('Doy' FROM now()),
  d NUMERIC DEFAULT EXTRACT('doy' FROM now()),
  e NUMERIC DEFAULT EXTRACT('user' FROM now())
);
-- @test
SELECT column_name, column_default FROM information_schema.columns
WHERE table_name = 'xd' AND column_default IS NOT NULL ORDER BY column_name
