-- Default (NO ACTION / RESTRICT): deleting a parent that still has children is
-- rejected.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY, parent_id INT REFERENCES fk_parent(id));
INSERT INTO fk_parent VALUES (1, 'a');
INSERT INTO fk_child VALUES (10, 1);

-- @test
-- @expect-error: a referenced parent cannot be deleted under NO ACTION
DELETE FROM fk_parent WHERE id = 1;
