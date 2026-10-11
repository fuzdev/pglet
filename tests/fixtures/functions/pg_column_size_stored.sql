-- `pg_column_size` of a stored column: the value is read from its heap row, where
-- `heap_form_tuple` packed a varlena of at most 126 data bytes to a one-byte header — so
-- a column's `'\xdeadbeef'` is 5 where the same constant is 8 — and a longer one keeps
-- four. Exact here because no row of the table ever reached the TOAST threshold.
-- @setup
CREATE TABLE pcs (
  id int, b bytea, ba bytea[], nb bytea[], t text, long_t text, i int, n numeric,
  u uuid, j jsonb, ia int[], ta text[], la int8[], bo bool, ts timestamptz,
  iv interval, d date
);
INSERT INTO pcs VALUES (
  1, '\xdeadbeef', ARRAY['\xdead'::bytea, '\xbeef'::bytea], ARRAY['\xdead'::bytea, NULL],
  'hello', 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx', 1, 12.5, '80be9f6a-7337-43cf-8027-93e55cdc2331',
  '{"a": 1, "bb": [true, null, "s"]}', ARRAY[1,2,3], ARRAY['a','bc'], ARRAY[1,2]::int8[],
  true, '2024-01-02 03:04:05+00', '1 day 2 hours', '2024-01-02'
);
INSERT INTO pcs (id) VALUES (2);
-- @test
SELECT id, pg_column_size(b) AS b, pg_column_size(ba) AS ba, pg_column_size(nb) AS nb,
       pg_column_size(t) AS t, pg_column_size(long_t) AS long_t, pg_column_size(i) AS i,
       pg_column_size(n) AS n, pg_column_size(u) AS u, pg_column_size(j) AS j,
       pg_column_size(ia) AS ia, pg_column_size(ta) AS ta, pg_column_size(la) AS la,
       pg_column_size(bo) AS bo, pg_column_size(ts) AS ts, pg_column_size(iv) AS iv,
       pg_column_size(d) AS d
FROM pcs ORDER BY id;
