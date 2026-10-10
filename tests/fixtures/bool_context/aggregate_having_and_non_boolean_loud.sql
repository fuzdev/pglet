-- `AND` reached through a grouping query's `HAVING`.
-- @setup
CREATE TABLE bcag3 (b TEXT);
INSERT INTO bcag3 VALUES ('x'), ('y');
-- @expect-error: argument of AND must be type boolean
-- @test
SELECT b FROM bcag3 GROUP BY b HAVING count(*) > 0 AND 1;
