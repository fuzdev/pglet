-- A `DROP NOT NULL` a block rolls back is undone with it: the `NOT NULL` stands, and the
-- NULL it would have admitted is `23502`.
-- @no-txn
-- @expect-error: the NOT NULL is back
-- @setup
DROP TABLE IF EXISTS dnr;
CREATE TABLE dnr (v int NOT NULL);
BEGIN;
ALTER TABLE dnr ALTER COLUMN v DROP NOT NULL;
ROLLBACK;
-- @test
INSERT INTO dnr VALUES (NULL);
