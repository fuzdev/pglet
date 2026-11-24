-- `LATERAL unnest(…)` is refused `0A000`: a FROM function reads its argument against the enclosing levels, never a sibling item (EDGES row 312); 18.7 answers.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @expect-error: LATERAL before a function in FROM not supported
-- @test
SELECT * FROM lp p CROSS JOIN LATERAL unnest(ARRAY[p.id, 7]) u ORDER BY 1, 3;
