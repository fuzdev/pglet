-- A correlated sub-plan that is empty for the outer row answers `false` / `true` for that
-- row whatever its left operand, NULL included.
-- @setup
CREATE TABLE l (k INT, x INT);
INSERT INTO l VALUES (1, NULL), (2, 1), (3, 2);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (2), (NULL);
-- @test
SELECT k,
  x IN (SELECT a FROM r WHERE a > coalesce(l.x, 100)) AS in_gt,
  x NOT IN (SELECT a FROM r WHERE a = l.x) AS nin_eq
FROM l ORDER BY k;
