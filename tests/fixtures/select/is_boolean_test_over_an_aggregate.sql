-- A boolean test whose operand is an **aggregate**, in both positions a grouping query
-- evaluates it: the projection and `HAVING`.
--
-- `HAVING count(*) > 0 IS TRUE` is textbook SQL. The control is the sibling
-- `NOT (count(*) > 1)`.
-- @setup
CREATE TABLE tb (id INT, b BOOL, i INT);
INSERT INTO tb VALUES (1, true, 10), (2, false, 20), (3, NULL, NULL);
-- @test
SELECT
  (count(*) > 1) IS TRUE AS projected,
  (max(i) IS NULL) IS TRUE AS nested_is_null,
  (sum(i) > 0) IS NOT FALSE AS not_false,
  (count(*) > 1) IS UNKNOWN AS never_unknown,
  NOT (count(*) > 1) AS control
FROM tb;
