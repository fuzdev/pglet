-- A literal compared with a `bytea` column is read as the column's type: over a table
-- holding the four bytes of `abcd` and the two bytes `\xabcd`, `'abcd'` finds the first.
-- @setup
CREATE TABLE bytea_input_c (id INT, b BYTEA);
INSERT INTO bytea_input_c VALUES
  (1, '\x61626364'), (2, '\xabcd'), (3, '\x20'), (4, '\x'), (5, '\x63616665'), (6, '\xcafe');
-- @test
SELECT
  (SELECT id FROM bytea_input_c WHERE b = 'abcd') AS eq_plain,
  (SELECT id FROM bytea_input_c WHERE 'cafe' = b) AS eq_plain_left,
  (SELECT id FROM bytea_input_c WHERE b = '\xabcd') AS eq_hex,
  (SELECT id FROM bytea_input_c WHERE b = ' ') AS eq_space,
  (SELECT id FROM bytea_input_c WHERE b = '') AS eq_empty,
  (SELECT count(*) FROM bytea_input_c WHERE b <> 'abcd') AS ne_plain,
  (SELECT count(*) FROM bytea_input_c WHERE b > 'b') AS gt_plain,
  (SELECT count(*) FROM bytea_input_c WHERE b BETWEEN 'a' AND 'd') AS between_plain,
  'abcd'::bytea = '\xabcd'::bytea AS casts_differ,
  'abcd'::bytea = '\x61626364' AS cast_beside_literal;
