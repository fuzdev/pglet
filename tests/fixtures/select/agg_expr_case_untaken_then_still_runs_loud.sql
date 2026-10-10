-- The untaken `THEN` of an arm whose condition is not constant: `count(*) > 5` is false
-- for the group, and the arm's `sum` still divides by the `b = 0` row.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN count(*) > 5 THEN sum(1/b) ELSE 0 END FROM u;
