-- Two computed branches of different numeric types are folded to 18.6's common type
-- and cast to it: `COALESCE(NULL, length('ab'), n + 1)` is `numeric`, the `ARRAY`
-- spelling `numeric[]` keeping `2.5`, and `COALESCE(i + 0, b + 0)` is `bigint` on
-- every row, the `int4` rows widened with it.
-- @setup
CREATE TABLE w (i INT, b BIGINT, n NUMERIC);
INSERT INTO w VALUES (NULL, 5000000000, 1.5), (7, 1, NULL);
-- @test
SELECT COALESCE(NULL, length('ab'), n + 1) AS c1,
  CASE WHEN i IS NULL THEN length('ab') ELSE n + 1 END AS c2,
  ARRAY[NULL, length('ab'), n + 1] AS c3, COALESCE(i + 0, b + 0) AS c4,
  COALESCE(length('ab'), n + 1) AS c5
FROM w ORDER BY b;
