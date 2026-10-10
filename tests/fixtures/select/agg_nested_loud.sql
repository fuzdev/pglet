-- An aggregate whose argument is another aggregate is 18.6's `42803`, raised at plan time
-- as its parse analysis raises it, never evaluated.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @expect-error: aggregate function calls cannot be nested
-- @expect-message
-- @test
SELECT avg(avg(i)) FROM t;
