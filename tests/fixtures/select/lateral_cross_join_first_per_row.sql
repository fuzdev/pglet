-- `CROSS JOIN LATERAL (subquery)`: the subquery names the left item's column, and runs once per left row — its own `ORDER BY … LIMIT 1` picks each row's first child.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, c.v FROM lp p CROSS JOIN LATERAL (SELECT v FROM lc WHERE lc.pid = p.id ORDER BY pos LIMIT 1) c ORDER BY 1;
