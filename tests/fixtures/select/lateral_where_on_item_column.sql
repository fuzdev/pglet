-- The statement's `WHERE` over a lateral item's computed column.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT * FROM lp p CROSS JOIN LATERAL (SELECT 'k' || p.name AS w) x WHERE x.w = 'kb';
