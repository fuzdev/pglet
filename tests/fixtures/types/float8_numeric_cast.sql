-- float8 -> numeric renders the double's OWN 15-significant-digit precision (Postgres
-- float8_numeric's `%.*g`/DBL_DIG), not the 16-17 digits its shortest round-trip
-- carries (`(1/3)::float8::numeric` = `0.333333333333333`, 15 threes, not 16). Trailing
-- zeros are dropped, so the display scale matches too.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);

-- @test
SELECT
  0.1::float8::numeric AS exact_01,
  1.5::float8::numeric AS exact_15,
  (1::float8 / 3::float8)::numeric AS third,
  (2::float8 / 3::float8)::numeric AS two_thirds,
  1.2345678901234567::float8::numeric AS fifteen_sig,
  123456789012345.67::float8::numeric AS big_round,
  1e20::float8::numeric AS big,
  'NaN'::float8::numeric AS nan
FROM t;
