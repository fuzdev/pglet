-- The same comparison through a primary key's index: the probe value is the string read as
-- `bytea`, so `'abcd'` finds the four-byte key and the unique check sees `'abcd'` and
-- `'\x61626364'` as one key.
-- @profile-lock
-- @setup
CREATE TABLE bytea_input_k (b BYTEA PRIMARY KEY, id INT);
INSERT INTO bytea_input_k VALUES
  ('\x61626364', 1), ('\xabcd', 2), ('\x20', 3), ('\x', 4), ('cafe', 5), ('\xcafe', 6);
INSERT INTO bytea_input_k VALUES ('abcd', 7), ('beef', 8) ON CONFLICT (b) DO NOTHING;
-- @test
SELECT
  (SELECT id FROM bytea_input_k WHERE b = 'abcd') AS eq_plain,
  (SELECT id FROM bytea_input_k WHERE b = 'cafe') AS eq_plain_written_plain,
  (SELECT id FROM bytea_input_k WHERE b = '\xabcd') AS eq_hex,
  (SELECT id FROM bytea_input_k WHERE b = ' ') AS eq_space,
  (SELECT id FROM bytea_input_k WHERE b = '\142\145\145\146') AS eq_octal,
  (SELECT count(*) FROM bytea_input_k WHERE b >= 'b') AS ge_plain,
  (SELECT count(*) FROM bytea_input_k) AS total;
