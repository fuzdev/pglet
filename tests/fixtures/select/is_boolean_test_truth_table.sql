-- `x IS [NOT] TRUE | FALSE | UNKNOWN` — Postgres's `BooleanTest`, over all three
-- inputs a `bool` column can hold. The whole point of the family is that the result
-- is never NULL: every cell here is `t` or `f`, including the NULL row's.
-- @setup
CREATE TABLE tb (id INT, b BOOL);
INSERT INTO tb VALUES (1, true), (2, false), (3, NULL);
-- @test
SELECT
  id,
  b IS TRUE AS is_true,
  b IS NOT TRUE AS is_not_true,
  b IS FALSE AS is_false,
  b IS NOT FALSE AS is_not_false,
  b IS UNKNOWN AS is_unknown,
  b IS NOT UNKNOWN AS is_not_unknown
FROM tb
ORDER BY id;
