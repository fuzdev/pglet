-- `concat` is not strict, so a `NULL` argument folds nothing: the aggregate runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT concat(NULL, sum(1/b)) FROM u;
