-- A volatile gen_random_uuid() default is evaluated per row, so every backfilled
-- existing row gets a distinct uuid (the distinct count is deterministic).
-- @setup
CREATE TABLE alter_addcol_uuid (id INT);
INSERT INTO alter_addcol_uuid VALUES (1), (2), (3);
ALTER TABLE alter_addcol_uuid ADD COLUMN u UUID DEFAULT gen_random_uuid();

-- @test
SELECT count(DISTINCT u) FROM alter_addcol_uuid;
