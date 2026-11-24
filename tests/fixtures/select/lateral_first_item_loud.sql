-- A lateral item first in the `FROM` has nothing to its left: a qualifier naming a later table is `42P01`.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: missing FROM-clause entry for table "p"
-- @test
SELECT * FROM LATERAL (SELECT p.id FROM lc) x, lp p;
