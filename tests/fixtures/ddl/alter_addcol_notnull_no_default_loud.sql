-- ADD COLUMN NOT NULL with no default on a non-empty table can't fill existing
-- rows (Postgres 23502), not a silent store of NULLs into a NOT NULL column.
-- @expect-error: existing rows contain null values
-- @setup
CREATE TABLE alter_addcol_nn (id INT);
INSERT INTO alter_addcol_nn VALUES (1);

-- @test
ALTER TABLE alter_addcol_nn ADD COLUMN e INT NOT NULL;
