-- `x [NOT] IN (SELECT …)` over every combination of a NULL / matching / non-matching left
-- operand and a sub-plan that is empty, holds a match, holds no match, or holds no match
-- beside a NULL. A NULL operand is NULL over any set with a row in it and false / true
-- over the empty one.
-- @setup
CREATE TABLE l (k INT, x INT);
INSERT INTO l VALUES (1, NULL), (2, 1), (3, 3);
CREATE TABLE e (a INT);
CREATE TABLE m (a INT);
INSERT INTO m VALUES (1);
CREATE TABLE mn (a INT);
INSERT INTO mn VALUES (1), (NULL);
CREATE TABLE nn (a INT);
INSERT INTO nn VALUES (NULL);
-- @test
SELECT k,
  x IN (SELECT a FROM e) AS in_e, x NOT IN (SELECT a FROM e) AS nin_e,
  x IN (SELECT a FROM m) AS in_m, x NOT IN (SELECT a FROM m) AS nin_m,
  x IN (SELECT a FROM mn) AS in_mn, x NOT IN (SELECT a FROM mn) AS nin_mn,
  x IN (SELECT a FROM nn) AS in_nn, x NOT IN (SELECT a FROM nn) AS nin_nn,
  NOT (x IN (SELECT a FROM e)) AS not_in_e, NOT (x IN (SELECT a FROM mn)) AS not_in_mn
FROM l ORDER BY k;
