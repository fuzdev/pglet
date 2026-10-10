-- `ALTER TABLE IF EXISTS` over a table that exists performs the action exactly as the
-- plain spelling does. The parser used to read `IF` as the table name and `EXISTS` as
-- an unknown action, so the whole statement was a tolerated no-op reporting success.
-- @setup
CREATE TABLE atie1 (a INT);
INSERT INTO atie1 VALUES (1);
ALTER TABLE IF EXISTS atie1 ADD COLUMN b INT DEFAULT 5;
-- @test
SELECT * FROM atie1;
