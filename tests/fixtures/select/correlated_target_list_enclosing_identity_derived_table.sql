-- A derived table and a CTE whose column is a scalar subquery naming an enclosing
-- level's column take that column's type, so the gates above them read `int4`: 18.6
-- answers `v > 9` and `sum(v)`, where a `text` column would be `42883` (`EDGES.md`
-- row 159).
-- @setup
CREATE TABLE eid_p (a INT, b INT);
CREATE TABLE eid_q (c INT);
INSERT INTO eid_p VALUES (1, 10), (2, 20);
INSERT INTO eid_q VALUES (5);
-- @test
WITH c AS (SELECT a, (SELECT eid_p.b FROM eid_q) AS v FROM eid_p)
SELECT x.a, x.v > 9 AS gt, x.v, (SELECT sum(v) FROM c) AS total, c.v * 2 AS doubled
  FROM (SELECT a, (SELECT eid_p.b FROM eid_q) AS v FROM eid_p) x JOIN c USING (a)
 ORDER BY x.a;
