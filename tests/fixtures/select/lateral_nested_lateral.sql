-- A lateral item whose subquery holds its own lateral join, the inner one naming both its left item and the outer query's.
-- @setup
CREATE TABLE lp (id int, name text);
CREATE TABLE lc (pid int, pos int, v text);
INSERT INTO lp VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO lc VALUES (1, 2, 'x'), (1, 1, 'y'), (2, 5, 'z');
-- @test
SELECT p.id, y.id AS qid FROM lp p CROSS JOIN LATERAL (SELECT q.id FROM lp q CROSS JOIN LATERAL (SELECT q.id + p.id AS w) z WHERE z.w > 3) y ORDER BY 1, 2;
