-- An `IN` list's strings beside a `bytea` column are each read as `bytea`; a string that is
-- no run of hex digits is as ordinary an item as one that is.
-- @setup
CREATE TABLE bytea_input_i (id INT, b BYTEA);
INSERT INTO bytea_input_i VALUES
  (1, '\x61626364'), (2, '\xabcd'), (3, '\x63616665'), (4, '\x6e6f74686578'), (5, '\x20');
-- @test
SELECT id FROM bytea_input_i WHERE b IN ('abcd', 'cafe', 'nothex', ' ', 'absent') ORDER BY id;
