-- A lateral join inside a scalar subquery, its item naming both its left item and the enclosing query's row.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, (SELECT sum(z.s) FROM lc CROSS JOIN LATERAL (SELECT lc.pos + p.id AS s) z) AS total FROM lp p ORDER BY 1;
