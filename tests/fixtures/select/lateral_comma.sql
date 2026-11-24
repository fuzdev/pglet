-- A comma-joined `LATERAL` item: every child row per parent, and a parent with none dropped.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, c.v FROM lp p, LATERAL (SELECT v FROM lc WHERE lc.pid = p.id) c ORDER BY 1, 2;
