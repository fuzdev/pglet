-- Casts to/from NUMERIC: text/int are exact, numeric→int rounds half-away,
-- numeric→float8 is the lossy double cast, and a rescale cast rounds.
-- @setup
CREATE TABLE t (id INT);
INSERT INTO t VALUES (1);

-- @test
SELECT
  '1234.5678'::numeric AS from_text,
  42::numeric AS from_int,
  2.5::numeric::int8 AS to_int_round,
  (1::numeric / 3)::float8 AS to_float,
  123.456::numeric AS keeps_scale
FROM t;
