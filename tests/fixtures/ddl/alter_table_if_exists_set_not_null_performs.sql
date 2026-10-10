-- `SET NOT NULL` under `IF EXISTS` is enforced: the `NULL` the no-op let through is
-- `23502`.
-- @setup
CREATE TABLE atie4 (a INT);
ALTER TABLE IF EXISTS atie4 ALTER COLUMN a SET NOT NULL;
-- @expect-error: the column is NOT NULL now
-- @test
INSERT INTO atie4 VALUES (NULL);
