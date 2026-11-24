-- A lateral item's column-alias list longer than its columns: `42P10`.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: table "x" has 1 columns available but 2 columns specified
-- @test
SELECT * FROM lp p CROSS JOIN LATERAL (SELECT p.id) x(a, b);
