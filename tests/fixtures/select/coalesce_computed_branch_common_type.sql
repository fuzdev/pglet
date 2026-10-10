-- A computed branch's type takes part in a `COALESCE`'s/`CASE`'s/`ARRAY`'s common type:
-- `COALESCE(i, f + 0)` is `double precision` and answers `2.5`, not the `2` a cast to its
-- `integer` sibling's type gives; a fold of two computed branches is declared by their
-- common type, not the first one's.
-- @setup
CREATE TABLE t (i INT, f FLOAT8, n NUMERIC);
INSERT INTO t VALUES (1, 1.5, 0.5), (NULL, 2.5, 1.25), (3, NULL, NULL);
-- @test
SELECT COALESCE(i + 0, f + 0) AS c1, COALESCE(i, f + 0) AS c2,
  CASE WHEN i IS NULL THEN 0 ELSE f + 0 END AS c3, COALESCE(i + 0, n + 0) AS c4,
  ARRAY[i + 0, f + 0] AS c5, COALESCE(-i, f) AS c6, COALESCE(i % 2, n) AS c7
FROM t ORDER BY f;
