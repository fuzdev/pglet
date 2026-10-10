-- numeric -> float4 is a SINGLE correctly-rounded decimal→real step (Postgres
-- numeric_float4 = numeric_out then float4in). A numeric→f64→f32 double-rounding
-- picks the wrong neighbour near an f32 tie: the `d` literal is `1.0000001` here
-- (single-rounded), not `1.0000002`. NaN maps to a real NaN.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);

-- @test
SELECT
  0.1::numeric::float4 AS rt_01,
  1.5::numeric::float4 AS rt_15,
  3.14::numeric::float4 AS rt_314,
  '1.00000017881393421514957253748434595763683319091796875'::numeric::float4 AS single_round,
  (-2.5)::numeric::float4 AS neg,
  'NaN'::numeric::float4 AS nan
FROM t;
