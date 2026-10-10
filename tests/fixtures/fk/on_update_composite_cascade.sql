-- Composite ON UPDATE CASCADE: updating a parent's (x, y) key repoints every child
-- whose full (a, b) tuple matched the old (x, y) to the new value.
-- @profile-lock
-- @setup
CREATE TABLE oucc_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE oucc_child (
  id INT PRIMARY KEY,
  a INT,
  b INT,
  FOREIGN KEY (a, b) REFERENCES oucc_parent (x, y) ON UPDATE CASCADE
);
INSERT INTO oucc_parent VALUES (1, 2), (3, 4);
INSERT INTO oucc_child VALUES (10, 1, 2), (11, 1, 2), (20, 3, 4);
UPDATE oucc_parent SET x = 5, y = 6 WHERE x = 1 AND y = 2;

-- @test
-- Children of (1, 2) now point at (5, 6); the (3, 4) child is unchanged.
SELECT id, a, b FROM oucc_child ORDER BY id;
