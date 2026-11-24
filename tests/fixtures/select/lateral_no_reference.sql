-- A `LATERAL` item that names nothing to its left, first in the `FROM` and after a table.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT * FROM LATERAL (SELECT 1 AS a) x CROSS JOIN lp ORDER BY 2;
