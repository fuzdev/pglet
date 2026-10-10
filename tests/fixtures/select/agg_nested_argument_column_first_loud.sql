-- A missing column inside the outer aggregate's own argument is transformed before the
-- outer call is checked: `42703`, not the nesting's `42803`.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @expect-error: column does not exist
-- @test
SELECT avg(nosuch + max(i)) FROM t;
