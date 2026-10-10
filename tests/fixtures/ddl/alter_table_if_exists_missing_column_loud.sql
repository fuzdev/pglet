-- `42703` under `IF EXISTS`: the table's clause does not reach the column.
-- @setup
CREATE TABLE atie10 (a INT);
-- @expect-error: no such column
-- @expect-message
-- @test
ALTER TABLE IF EXISTS atie10 DROP COLUMN nosuch;
