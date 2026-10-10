-- A `text` value that is no literal — a column, a concatenation, a cast chain — converts to
-- `bytea` through the same input function.
-- @setup
CREATE TABLE bytea_input_src (id INT, t TEXT, v VARCHAR(20));
INSERT INTO bytea_input_src VALUES
  (1, 'abcd', 'cafe'), (2, '\x6162 63', '\x'), (3, ' ', 'a\\b'), (4, '\141', '  12'), (5, NULL, NULL);
-- @test
SELECT id, t::bytea AS from_text, v::bytea AS from_varchar, (t || v)::bytea AS from_concat,
  'abcd'::text::bytea AS chain, CAST('be' || 'ef' AS bytea) AS cast_concat
FROM bytea_input_src ORDER BY id;
