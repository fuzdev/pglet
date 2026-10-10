-- The **write** side of column-name exactness, and the one that used to be silent:
-- `resolve_insert_column` had a case-insensitive fallback after its exact match, so an
-- unquoted `userid` in the target list wrote to a `"UserId"` column that the matching
-- `SELECT userid FROM …` refused — pglet answered two different questions about one
-- column name depending on which side of the statement it appeared on. 18.6 raises
-- `42703` from `checkInsertTargets`, and so does pglet now.
-- @setup
CREATE TABLE qcol ("UserId" INT, v INT);

-- @test
INSERT INTO qcol (userid, v) VALUES (1, 1);
-- @expect-error: the column is named `UserId`, and `userid` is a different name (42703)
