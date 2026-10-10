-- The `DROP COLUMN` action under `ALTER TABLE IF EXISTS … ONLY`, the spelling `pg_dump`
-- writes, over a table that exists.
-- @setup
CREATE TABLE atie2 (a INT, b INT);
INSERT INTO atie2 VALUES (1, 2);
ALTER TABLE IF EXISTS ONLY atie2 DROP COLUMN b;
-- @test
SELECT * FROM atie2;
