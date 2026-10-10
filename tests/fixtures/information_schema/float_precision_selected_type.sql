-- The catalog reports the **selected** type, not `float` plus a modifier: a `FLOAT(1)`
-- column is `real`/`float4`/24 and a `FLOAT(25)` one is `double precision`/`float8`/53,
-- each indistinguishable from the spelling that names that type directly (`f`/`g` are
-- those controls). `numeric_precision` is the *type's* — neither float type carries an
-- `atttypmod` — so a build that kept the modifier instead of selecting on it would have
-- to invent a value here.
--
-- The array column takes the selection at its **element**, so it reports `ARRAY` over
-- `_float4` rather than `_float8`.
-- @no-declared-types: `column_name` and `udt_name` are `sql_identifier` (base `name`,
--   OID 19) and `data_type` is `character_data` (base `varchar`, 1043) on 18.6, where
--   pglet declares `text` (25) for all three — measured by removing this waiver, and
--   exactly those three. `numeric_precision` itself agrees. What pglet has no model of
--   is a per-column declared type for a catalog view, which is
--   `information_schema/precision_datetime_precision.sql`'s waiver too.
-- @setup
CREATE TABLE fs (
  a FLOAT(1),
  b FLOAT(24),
  c FLOAT(25),
  d FLOAT(53),
  e FLOAT,
  f real,
  g double precision,
  h FLOAT(1)[]
);
-- @test
SELECT column_name, data_type, udt_name, numeric_precision
  FROM information_schema.columns
 WHERE table_name = 'fs'
 ORDER BY ordinal_position;
