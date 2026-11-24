-- A lateral item comparing its left item's `text` column with a number: `42883`, over every row.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: operator does not exist: text = integer
-- @test
SELECT * FROM lp p CROSS JOIN LATERAL (SELECT 1 WHERE p.name = 1) x;
