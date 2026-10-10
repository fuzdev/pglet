-- nullif resolves the common type of its two arguments: nullif(5, 6.0) returns a
-- numeric 5 (not an int); a NULL argument returns the first argument (coerced to
-- the common type); nullif(x, x) is NULL.
-- @setup
CREATE TABLE nt (id INT, a INT, n NUMERIC);
INSERT INTO nt VALUES (1, 5, 6.0), (2, 5, 5.0), (3, 5, NULL);
-- @test
SELECT id,
  nullif(a, n) AS an,
  nullif(5, 6.0) AS lit,
  nullif(a::int8, 5::int4) AS widen
FROM nt ORDER BY id;
