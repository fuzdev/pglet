-- A derived table over a set operation naming the enclosing `numeric` column carries that
-- type into the scalar subquery reading it, and `COALESCE` beside `2` is `numeric`.
-- @setup
CREATE TABLE sodt (n NUMERIC);
INSERT INTO sodt VALUES (1.25);
-- @test
SELECT coalesce((SELECT s.n FROM (SELECT n EXCEPT SELECT '7') s LIMIT 1), 2) AS c,
       greatest((SELECT s.n FROM (SELECT n EXCEPT SELECT '7') s LIMIT 1), 2) AS g
FROM sodt;
