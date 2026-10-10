-- ON UPDATE CASCADE: updating a parent's key repoints every referencing child's FK
-- column to the new value.
-- @profile-lock
-- @setup
CREATE TABLE ouc_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE ouc_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES ouc_parent(id) ON UPDATE CASCADE
);
INSERT INTO ouc_parent VALUES (1, 'a'), (2, 'b');
INSERT INTO ouc_child VALUES (10, 1), (11, 1), (20, 2);
UPDATE ouc_parent SET id = 100 WHERE id = 1;

-- @test
-- Children of parent 1 now point at 100; parent 2's child is unchanged.
SELECT id, parent_id FROM ouc_child ORDER BY id;
