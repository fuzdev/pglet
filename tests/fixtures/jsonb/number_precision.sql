-- Postgres stores jsonb numbers as NUMERIC (arbitrary precision), not f64, so a
-- 30-digit integer and a >17-significant-digit decimal round-trip EXACTLY rather
-- than being f64-rounded. Regression guard for the `JsonbValue::Number`
-- f64→PgNumeric change (store magic PGMC). An f64 would render the big integer as
-- `123…680000…` and the decimal as `0.12345678901234568`.
-- @test
SELECT
  '123456789012345678901234567890'::jsonb                    AS big_int,
  ('{"n": 123456789012345678901234567890}'::jsonb) ->> 'n'   AS big_int_extract,
  '100000000000000000000'::jsonb                             AS twenty_one_digits,
  '9223372036854775808'::jsonb                               AS past_i64,
  '0.1234567890123456789'::jsonb                             AS high_scale_decimal,
  ('{"n": 0.1234567890123456789}'::jsonb) ->> 'n'            AS high_scale_extract;
