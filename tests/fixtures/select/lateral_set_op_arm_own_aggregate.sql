-- An arm's aggregate over its own columns stays in the arm, beside an arm naming the left
-- item.
-- @setup
CREATE TABLE lp (id int);
CREATE TABLE lc (pid int);
INSERT INTO lp VALUES (1), (2);
INSERT INTO lc VALUES (1);
-- @test
SELECT p.id, s.* FROM lp p, LATERAL (SELECT count(pid) AS c FROM lc UNION ALL SELECT p.id) s ORDER BY 1, 2;
