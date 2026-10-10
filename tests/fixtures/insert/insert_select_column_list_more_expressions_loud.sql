-- The `INSERT … SELECT` door under a column list, the source longer than the list:
-- `42601 INSERT has more expressions than target columns`.
-- @setup
CREATE TABLE iscx (a INT, b INT);

-- @test
-- @expect-error: INSERT has more expressions than target columns (42601)
-- @expect-message
INSERT INTO iscx (a) SELECT 1, 2;
