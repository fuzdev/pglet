-- power / pow are the `^` operator as calls: float8 over integers and floats (no
-- integer or real result), numeric when a numeric meets an integer or numeric; a NULL
-- argument is NULL, and a bare NULL beside an integer or a float is typed by the call.
-- @setup
CREATE TABLE pw (id INT, i INT, s SMALLINT, g BIGINT, n NUMERIC, f REAL, d DOUBLE PRECISION);
INSERT INTO pw VALUES
  (1, 2, 3, 4, 1.5, 2.5, 0.5),
  (2, -2, 2, 1, 2.0, -1, 9),
  (3, NULL, NULL, NULL, NULL, NULL, NULL);
-- @test
SELECT id,
  power(i, s) AS ii, pow(g, i) AS gi, power(n, i) AS ni, power(i, n * 0 + 2) AS in_,
  power(f, i) AS fi, power(d, 0.5) AS df, power(n, s) AS ns,
  power(i, NULL) AS inull, pow(NULL, d) AS nulld
FROM pw ORDER BY id;
