-- The same with the enclosing column named bare.
-- @setup
CREATE TABLE ev (id INT, n INT, g INT);
INSERT INTO ev VALUES (1, 10, 1), (2, 20, 1), (3, 30, 2);
CREATE TABLE o (y INT, z FLOAT8);
INSERT INTO o VALUES (1, 0.5), (2, 1.5);
-- @test
-- @expect-error: the literal stays text beside an enclosing-level aggregate
SELECT (SELECT count(z) = '2' FROM ev LIMIT 1) FROM o;
