-- An inline UNIQUE on the added column is enforced: a later duplicate insert is
-- 23505, not silently accepted.
-- @expect-error: duplicate value violates the ALTER-added UNIQUE
-- @profile-lock
-- @setup
CREATE TABLE alter_addcol_uq (id INT);
INSERT INTO alter_addcol_uq VALUES (1), (2);
ALTER TABLE alter_addcol_uq ADD COLUMN email TEXT UNIQUE;
INSERT INTO alter_addcol_uq VALUES (3, 'x');

-- @test
INSERT INTO alter_addcol_uq VALUES (4, 'x');
