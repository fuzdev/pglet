-- An inline CHECK on the added column is enforced: a violating insert is 23514.
-- @expect-error: value violates the ALTER-added CHECK
-- @setup
CREATE TABLE alter_addcol_ck (id INT);
INSERT INTO alter_addcol_ck VALUES (1);
ALTER TABLE alter_addcol_ck ADD COLUMN val INT CHECK (val > 0);

-- @test
INSERT INTO alter_addcol_ck VALUES (2, -5);
