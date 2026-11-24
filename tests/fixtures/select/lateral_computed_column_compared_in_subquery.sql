-- A lateral item inside an `EXISTS` body, its computed column compared with a number: the column is typed from the left item's before the body's `WHERE` is.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT * FROM lp p WHERE EXISTS (SELECT 1 FROM lp q CROSS JOIN LATERAL (SELECT q.id + p.id AS w) z WHERE z.w = 3) ORDER BY 1;
