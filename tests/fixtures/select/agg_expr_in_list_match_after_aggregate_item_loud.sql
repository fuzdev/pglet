-- An `IN` list drops nothing, wherever a constant item equal to the operand stands: 18.6
-- gathers these items into one array behind the `count(*)`, so the `sum` after the
-- matching `1` is computed there too. Stopping the collection at the matching item
-- would answer `t`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT 1 IN (count(*), 1, sum(1/b)) FROM u;
