-- `42703` from `ALTER COLUMN … SET DEFAULT` under `IF EXISTS`.
-- @setup
CREATE TABLE atie12 (a INT);
-- @expect-error: no such column
-- @expect-message
-- @test
ALTER TABLE IF EXISTS atie12 ALTER COLUMN nosuch SET DEFAULT 1;
