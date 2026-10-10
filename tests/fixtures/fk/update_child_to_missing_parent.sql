-- Repointing a child's FK column at a non-existent parent violates the FK.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY, parent_id INT REFERENCES fk_parent(id));
INSERT INTO fk_parent VALUES (1, 'a');
INSERT INTO fk_child VALUES (10, 1);

-- @test
-- @expect-error: update repoints the FK at a missing parent
UPDATE fk_child SET parent_id = 99 WHERE id = 10;
