-- A window key equal to the source's constant resolves it to `text`, so the assignment
-- into a `boolean` column is refused as it is under an `ORDER BY` naming the constant.
-- @setup
CREATE TABLE awk_src (a INT);
INSERT INTO awk_src VALUES (7);
CREATE TABLE awk_dst (bo BOOL, b BIGINT);
-- @expect-error: column "bo" is of type boolean but expression is of type text
-- @test
INSERT INTO awk_dst (bo, b) SELECT 't', row_number() OVER (PARTITION BY 't') FROM awk_src;
