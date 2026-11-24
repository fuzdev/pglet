-- `LATERAL (VALUES …)` is refused `0A000` rather than read without its left item; 18.7 answers.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: LATERAL before a VALUES list not supported
-- @test
SELECT * FROM lp p CROSS JOIN LATERAL (VALUES (p.id)) x(a) ORDER BY 1;
