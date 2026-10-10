-- mod is the `%` operator as a call: the integer widths keep the wider, an integer
-- beside a numeric is numeric, the sign follows the dividend; NULL is NULL; a string
-- literal beside an integer literal resolves to its type.
-- @setup
CREATE TABLE md (id INT, i INT, s SMALLINT, g BIGINT, n NUMERIC);
INSERT INTO md VALUES
  (1, 7, 3, 10, 7.5),
  (2, -7, 3, -10, -7.5),
  (3, 7, -3, 9223372036854775807, 0.3),
  (4, NULL, NULL, NULL, NULL);
-- @test
SELECT id,
  mod(i, s) AS is_, mod(s, s) AS ss, mod(g, i) AS gi, mod(i, g) AS ig,
  mod(n, 2) AS n2, mod(i, n) AS in_, mod(i, 3.5) AS i35,
  mod(i, NULL) AS inull
FROM md ORDER BY id;
