-- A scalar subquery in the target list is built before the enclosing `FROM` is read; its
-- set operation's column still takes the enclosing column's `integer`.
-- @setup
CREATE TABLE dst (i INT);
INSERT INTO dst VALUES (7);
-- @param-declared $1 int4 5
-- @test
SELECT (SELECT $1 UNION ALL SELECT i LIMIT 1) AS v FROM dst;
