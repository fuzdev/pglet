-- A `timestamptz[]` element's text carries its `+00`, as a scalar's does, on each of
-- these routes that build or render one: an array literal, the `ARRAY[…]` constructor, a
-- stored column, a `::text` cast, a `::text[]` or `::varchar[]` cast, `concat`,
-- `concat_ws`, and a `COALESCE`. Without the offset a
-- text-protocol driver reads the element in its own zone. The `timestamp[]` column is
-- the control, rendered without one.
-- @setup
CREATE TABLE tza (id INT, a TIMESTAMPTZ[], b TIMESTAMP[]);
INSERT INTO tza VALUES
  (1, '{"2000-01-01 00:00:00+05",NULL,infinity,-infinity}', '{"2000-01-01 00:00:00+05",NULL}'),
  (2, ARRAY[timestamptz '2001-06-15 12:30:45.5-03:30'], ARRAY[timestamp '2001-06-15 12:30:45.5']);
-- @test
SELECT
  id,
  a,
  b,
  a::text AS a_text,
  concat(a) AS a_concat,
  COALESCE(NULL, a) AS a_coalesce,
  '{"2000-01-01 00:00:00+05"}'::timestamptz[] AS literal,
  ARRAY[timestamptz '2000-01-01 00:00:00+05', NULL] AS constructed,
  ARRAY[timestamptz '2000-01-01 00:00:00+05']::text AS constructed_text,
  concat(timestamptz '2000-01-01 00:00:00+05') AS scalar_concat,
  concat_ws('|', a, b, timestamptz '2000-01-01 00:00:00+05', timestamp '2000-01-01 00:00:00') AS concat_ws,
  a::text[] AS a_text_array,
  ARRAY[timestamptz '2000-01-01 00:00:00+05']::varchar[] AS constructed_varchar_array
FROM tza ORDER BY id;
