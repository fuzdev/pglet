-- ON DELETE CASCADE: deleting a parent removes its referencing children too.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES fk_parent(id) ON DELETE CASCADE
);
INSERT INTO fk_parent VALUES (1, 'a'), (2, 'b');
INSERT INTO fk_child VALUES (10, 1), (11, 1), (20, 2);
DELETE FROM fk_parent WHERE id = 1;

-- @test
-- Children of parent 1 are gone; parent 2's child survives.
SELECT id, parent_id FROM fk_child ORDER BY id;
