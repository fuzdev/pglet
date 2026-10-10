-- `SET DEFAULT` and `DROP DEFAULT` under `IF EXISTS` — the second is what
-- `pg_dump --clean --if-exists` emits for a `SERIAL` column ahead of its `DROP SEQUENCE`.
-- @setup
CREATE TABLE atie3 (a INT, b INT, c INT DEFAULT 9);
ALTER TABLE IF EXISTS atie3 ALTER COLUMN b SET DEFAULT 7;
ALTER TABLE IF EXISTS atie3 ALTER COLUMN c DROP DEFAULT;
INSERT INTO atie3 (a) VALUES (1);
-- @test
SELECT * FROM atie3;
