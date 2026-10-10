-- The `INSERT … SELECT` door of the long row: the source produces more columns than the
-- table has — `42601 INSERT has more expressions than target columns`.
-- @setup
CREATE TABLE isme (i INT);

-- @test
-- @expect-error: INSERT has more expressions than target columns (42601)
-- @expect-message
INSERT INTO isme SELECT 1, 2;
