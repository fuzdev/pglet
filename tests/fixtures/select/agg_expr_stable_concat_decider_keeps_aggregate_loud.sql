-- `concat` is stable in 18.6 (the `||` operator over strings is not), so a condition
-- calling it is no constant.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN concat('a', 'b') = 'ab' THEN 0 ELSE sum(1/b) END FROM u;
