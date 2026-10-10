-- `greatest`/`least` take a computed argument's type into their common type, and
-- `nullif` its first argument's: `least(i, f + 0)` answers `2.5`, `greatest(i, n * 2)`
-- `2.50`, `nullif(f + 0, i)` `1.5`.
-- @setup
CREATE TABLE t (i INT, f FLOAT8, n NUMERIC);
INSERT INTO t VALUES (1, 1.5, 0.5), (NULL, 2.5, 1.25), (3, NULL, NULL);
-- @test
SELECT greatest(i + 0, f + 0) AS g1, least(i, f + 0) AS l1, greatest(i, n * 2) AS g2,
  nullif(f + 0, i) AS n1, nullif(i * 2, 2) AS n2
FROM t ORDER BY f;
