-- `count` over an enclosing query's column is 18.6's enclosing level's aggregate (`EDGES.md` row 303); the
-- literal is not read at `bigint` there, which would answer row 303's wrong rows.
-- @setup
CREATE TABLE ev (id INT, n INT, g INT);
INSERT INTO ev VALUES (1, 10, 1), (2, 20, 1), (3, 30, 2);
CREATE TABLE o (y INT, z FLOAT8);
INSERT INTO o VALUES (1, 0.5), (2, 1.5);
-- @test
-- @expect-error: the literal stays text beside an enclosing-level aggregate
SELECT (SELECT count(o.z) = '2' FROM ev LIMIT 1) FROM o;
