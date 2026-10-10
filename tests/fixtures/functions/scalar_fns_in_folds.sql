-- The new functions as branches of COALESCE / CASE / greatest: a branch takes the type
-- the function answers (float8 for power, int4 for octet_length, numeric for mod over
-- numeric, bytea for substr over bytea).
-- @setup
CREATE TABLE uf (id INT, a INT, n NUMERIC, t TEXT, b BYTEA);
INSERT INTO uf VALUES (1, 3, 7.5, 'abc', '\x6162'), (2, NULL, NULL, NULL, NULL);
-- @test
SELECT id,
  COALESCE(power(a, 2), 0) AS c_pow,
  COALESCE(octet_length(t), 0) AS c_ol,
  COALESCE(mod(n, 2), 0) AS c_mod,
  COALESCE(mod(a, 2), 1.5) AS c_mod_num,
  CASE WHEN a > 0 THEN power(a, 0.5::float8) ELSE 1 END AS case_pow,
  greatest(mod(a, 3), 1.5) AS g_mod,
  COALESCE(substr(b, 1, 1), '\x00'::bytea) AS c_sub,
  COALESCE(left(t, 1), 'z') AS c_left
FROM uf ORDER BY id;
