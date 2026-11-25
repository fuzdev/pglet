-- ...and under the set operation's own trailing `ORDER BY`.
-- @setup
CREATE TABLE lp (id int);
CREATE TABLE lc (pid int);
INSERT INTO lp VALUES (1), (2);
INSERT INTO lc VALUES (1);
-- @expect-error: aggregate functions are not allowed in FROM clause of their own query level
-- @test
SELECT p.id, s.* FROM lp p, LATERAL ((SELECT count(p.id) AS c FROM lc) UNION ALL (SELECT 1) ORDER BY 1) s ORDER BY 1;
