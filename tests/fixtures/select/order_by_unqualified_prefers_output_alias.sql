-- @setup
CREATE TABLE t1 (id INT, x INT);
CREATE TABLE t2 (id INT, y INT);
INSERT INTO t1 VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO t2 VALUES (1, 3), (2, 2), (3, 1);

-- A *simple name* matching both an output column name and an input column name is
-- the output column (select.sgml). Here `y` is both the alias of t1.x and a base
-- column of t2, and the two sort in opposite orders — so the precedence rule is
-- observable. This must keep working: the qualifier-aware sort placement narrows
-- what routes below the projection, and an unqualified alias key must not.
-- @test
SELECT t1.x AS y FROM t1 JOIN t2 ON t1.id = t2.id ORDER BY y;
