-- ON UPDATE SET NULL: updating a parent's key nulls the referencing children's FK
-- columns.
-- @profile-lock
-- @setup
CREATE TABLE ousn_parent (id INT PRIMARY KEY);
CREATE TABLE ousn_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES ousn_parent(id) ON UPDATE SET NULL
);
INSERT INTO ousn_parent VALUES (1), (2);
INSERT INTO ousn_child VALUES (10, 1), (11, 1), (20, 2);
UPDATE ousn_parent SET id = 100 WHERE id = 1;

-- @test
-- Children of parent 1 are nulled; parent 2's child is unchanged.
SELECT id, parent_id FROM ousn_child ORDER BY id;
