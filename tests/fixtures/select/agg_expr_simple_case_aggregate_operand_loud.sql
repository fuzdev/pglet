-- A simple `CASE` whose operand is an aggregate is not constant, so no arm folds and the
-- `ELSE` aggregate runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE count(*) WHEN 2 THEN 0 ELSE sum(1/b) END FROM u;
