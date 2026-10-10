-- A set operation over a scalar subquery naming an enclosing level's `int` column unifies
-- the `'9'` beside it as `int4` and deduplicates the `9`s — two rows, numerically
-- ordered. Read as `text`, it would answer `9`, `10`, `9` (`EDGES.md` row 159).
-- @setup
CREATE TABLE eid_pn (a INT, b INT);
CREATE TABLE eid_q (c INT);
INSERT INTO eid_pn VALUES (1, 10), (2, 9);
INSERT INTO eid_q VALUES (5);
-- @test
SELECT (SELECT eid_pn.b FROM eid_q) AS v FROM eid_pn UNION SELECT '9' ORDER BY 1;
