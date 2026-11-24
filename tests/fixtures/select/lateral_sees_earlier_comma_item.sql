-- A lateral item inside a later comma item's join chain sees the earlier comma item too, and its computed column takes the left columns' types.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT * FROM lp p, lp q CROSS JOIN LATERAL (SELECT p.id + q.id AS s, p.name || q.name AS n) c ORDER BY 1, 3;
