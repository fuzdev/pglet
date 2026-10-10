-- The same rule from the other side: a table named to `DROP INDEX` is `42809`
-- (`"x" is not an index`), where pglet reported `42704`.
-- @setup
CREATE TABLE diot_t (a INT);
-- @test
DROP INDEX diot_t;
-- @expect-error: a table is not an index (42809)
