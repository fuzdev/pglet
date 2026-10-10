-- Self-referential ON UPDATE CASCADE: updating an interior node's key cascades to
-- the rows that reference it (the updated node does not reference the old key).
-- @profile-lock
-- @setup
CREATE TABLE ousr_node (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES ousr_node(id) ON UPDATE CASCADE
);
INSERT INTO ousr_node VALUES (1, NULL);
INSERT INTO ousr_node VALUES (2, 1), (3, 1);
UPDATE ousr_node SET id = 100 WHERE id = 1;

-- @test
-- The root is now 100 and its children point at 100.
SELECT id, parent_id FROM ousr_node ORDER BY id;
