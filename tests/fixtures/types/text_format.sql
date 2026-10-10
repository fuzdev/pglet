-- @test
-- @text
-- Compares the text-format wire output (simple query protocol) against Postgres,
-- not the binary path. Guards the scalar text encoders: bytea hex, timestamp
-- (space form, trimmed fractional), float infinity, plus a normal float and bool.
SELECT
  '\xDEADBEEF'::bytea AS b,
  '2024-06-15 12:30:45.123'::timestamp AS ts,
  'Infinity'::float8 AS inf,
  1.5::float8 AS f,
  true AS flag;
