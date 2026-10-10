-- A NULL left operand of an `IN` list whose items evaluate cleanly is NULL, for `IN` and
-- `NOT IN`; a matching non-NULL operand stops at its match, so a later item's division
-- does not run.
-- @setup
CREATE TABLE l (k INT, x INT, y INT);
INSERT INTO l VALUES (1, NULL, 1), (2, 1, 0);
-- @test
SELECT k, x IN (1, 2) AS i, x NOT IN (1, y) AS ni, x IN (1, 1/y) AS match_first
FROM l ORDER BY k;
