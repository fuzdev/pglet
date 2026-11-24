-- A no-FROM select with a `WHERE` reading an enclosing query's column, as the subquery of
-- an `EXISTS`.
-- @setup
CREATE TABLE nfw (a int);
INSERT INTO nfw VALUES (1), (3), (5);
-- @test
SELECT a FROM nfw WHERE EXISTS (SELECT 1 WHERE nfw.a > 2) ORDER BY a;
