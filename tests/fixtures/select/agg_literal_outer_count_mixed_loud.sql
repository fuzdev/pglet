-- An argument naming its own level's column and an enclosing one: the enclosing one does not bind here.
-- @setup
CREATE TABLE ev (id INT, n INT, g INT);
INSERT INTO ev VALUES (1, 10, 1), (2, 20, 1), (3, 30, 2);
CREATE TABLE o (y INT, z FLOAT8);
INSERT INTO o VALUES (1, 0.5), (2, 1.5);
-- @test
-- @expect-error: the literal stays text beside an argument with an enclosing column
SELECT (SELECT count(o.z + n) = '3' FROM ev) FROM o;
