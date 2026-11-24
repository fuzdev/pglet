-- The same outer-level aggregate in the lateral subquery's `HAVING`: `42803`.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: aggregate functions are not allowed in FROM clause of their own query level
-- @test
SELECT p.id, s.* FROM lp p, LATERAL (SELECT sum(pos) AS c FROM lc WHERE pid = p.id HAVING max(p.id) > 1) s ORDER BY 1;
