-- float4 -> numeric renders the float4's OWN 6-significant-digit precision
-- (Postgres float4_numeric's `%.*g`/FLT_DIG), not the f64-widened noise a
-- `f64::from(n)` capture would give (`0.1::float4::numeric` = `0.1`, not
-- `0.10000000149…`). Trailing zeros are dropped, so the display scale matches too.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);

-- @test
SELECT
  0.1::float4::numeric AS exact_01,
  3.14::float4::numeric AS exact_314,
  1.5::float4::numeric AS exact_15,
  0.3::float4::numeric AS exact_03,
  123456.7::float4::numeric AS six_sig_round,
  8388609::float4::numeric AS six_sig_int,
  2.0000002::float4::numeric AS six_sig_drop,
  1e20::float4::numeric AS big,
  'NaN'::float4::numeric AS nan
FROM t;
