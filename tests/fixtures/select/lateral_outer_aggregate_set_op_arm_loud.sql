-- An outer-level aggregate in either arm of a set operation in a lateral subquery is
-- 18.7's `42803`, each arm judged as its own query level.
-- @setup
CREATE TABLE lp (id int);
CREATE TABLE lc (pid int);
INSERT INTO lp VALUES (1), (2);
INSERT INTO lc VALUES (1);
-- @expect-error: aggregate functions are not allowed in FROM clause of their own query level
-- @test
SELECT p.id, s.* FROM lp p, LATERAL (SELECT 1 UNION ALL SELECT count(p.id) FROM lc) s ORDER BY 1;
