-- The parenthesized `IN` sub-select is the sub-select form under correlation too: empty
-- for an outer row, `NOT IN` is `true` for that row whatever its operand.
-- @setup
CREATE TABLE l (k INT, x INT);
INSERT INTO l VALUES (1, NULL), (2, 1), (3, 2);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (2);
-- @test
SELECT k, x NOT IN ((SELECT a FROM r WHERE a = l.x)) AS nin
FROM l ORDER BY k;
