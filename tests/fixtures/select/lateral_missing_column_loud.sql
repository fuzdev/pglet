-- A lateral item naming a column its left item lacks: 18.7's `42703`.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: column p.idd does not exist
-- @test
SELECT * FROM lp p CROSS JOIN LATERAL (SELECT p.idd) x;
