-- The nested aggregate is raised as its target-list entry is transformed, so it outranks a
-- missing column in a later entry, and a `WHERE` clause's, which 18.6 transforms after the
-- target list.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @expect-error: aggregate function calls cannot be nested
-- @expect-message
-- @test
SELECT avg(avg(i)), nosuch FROM t WHERE nosuch2 > 0;
