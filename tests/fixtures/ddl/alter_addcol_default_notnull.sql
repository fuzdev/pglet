-- ADD COLUMN … DEFAULT … NOT NULL backfills existing rows and stays non-null.
-- @setup
CREATE TABLE alter_addcol_dnn (id INT);
INSERT INTO alter_addcol_dnn VALUES (1), (2);
ALTER TABLE alter_addcol_dnn ADD COLUMN d INT DEFAULT 9 NOT NULL;

-- @test
SELECT id, d FROM alter_addcol_dnn ORDER BY id;
