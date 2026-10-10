-- A missing column in an earlier target-list entry is transformed first and wins: `42703`.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @expect-error: column does not exist
-- @test
SELECT nosuch, avg(avg(i)) FROM t;
