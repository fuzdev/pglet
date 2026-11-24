-- An aggregate in the lateral subquery is computed per left row, `0` for a row with no child.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, c.n FROM lp p CROSS JOIN LATERAL (SELECT count(*) AS n FROM lc WHERE pid = p.id) c ORDER BY 1;
