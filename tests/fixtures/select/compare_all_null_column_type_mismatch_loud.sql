-- The all-NULL twin of `compare_empty_indexed_table_type_mismatch_loud.sql`: the
-- table has rows, but every value of the compared column is NULL. A NULL carries no
-- type, so the eval-time gate passed it and NULL-propagated for every row — zero rows
-- returned where Postgres raises `42883`. The static gate reads the *declared* type
-- and is unaffected by the data.
-- @setup
CREATE TABLE t (name TEXT, n INT);
INSERT INTO t VALUES (NULL, NULL), (NULL, NULL);
-- @test
-- @expect-error: no text = integer operator, even when every row is NULL
SELECT 1 FROM t WHERE name = 5;
