-- ADD COLUMN with a constant DEFAULT backfills every existing row (Postgres fills
-- them, not NULL). A later insert with no value for the column still resolves it.
-- @setup
CREATE TABLE alter_addcol_def (id INT);
INSERT INTO alter_addcol_def VALUES (1), (2), (3);
ALTER TABLE alter_addcol_def ADD COLUMN c INT DEFAULT 5;
INSERT INTO alter_addcol_def (id) VALUES (4);

-- @test
SELECT id, c FROM alter_addcol_def ORDER BY id;
