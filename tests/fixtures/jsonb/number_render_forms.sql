-- jsonb renders a number with Postgres's `numeric_out` (never scientific): `1e2`
-- becomes `100` (NOT `1e+2`), trailing zeros are preserved (`1.0`, `1.00`), `-0`
-- normalizes to `0`. Locks the PgNumeric `to_display_string` render byte-for-byte
-- against Postgres across the number spellings. Each column is a jsonb scalar, so
-- the object-key-ordering divergence does not apply.
-- @test
SELECT
  '1.0'::jsonb    AS one_point_zero,
  '1.00'::jsonb   AS one_point_two_zeros,
  '1e2'::jsonb    AS sci_lower,
  '1E2'::jsonb    AS sci_upper,
  '1.5e-3'::jsonb AS sci_negative_exp,
  '1.20e2'::jsonb AS sci_trailing_zero,
  '10e-1'::jsonb  AS sci_to_one_scale,
  '-0'::jsonb     AS negative_zero,
  '0.0'::jsonb    AS zero_point_zero,
  '-42.5'::jsonb  AS negative_decimal;
