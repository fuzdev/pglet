-- An aggregate in a lateral subquery whose argument mixes the subquery's own column with its left item's is the subquery's own, evaluated per left row.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, s.* FROM lp p, LATERAL (SELECT max(p.id + lc.pos) AS c FROM lc) s ORDER BY 1;
