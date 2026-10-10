-- Composite FOREIGN KEY with ON DELETE CASCADE: deleting a parent removes every
-- child whose full (a, b) tuple equals the deleted (x, y).
-- @profile-lock
-- @setup
CREATE TABLE cdc_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE cdc_child (
  id INT PRIMARY KEY,
  a INT,
  b INT,
  FOREIGN KEY (a, b) REFERENCES cdc_parent (x, y) ON DELETE CASCADE
);
INSERT INTO cdc_parent VALUES (1, 2), (3, 4);
INSERT INTO cdc_child VALUES (10, 1, 2), (11, 1, 2), (20, 3, 4);
DELETE FROM cdc_parent WHERE x = 1 AND y = 2;

-- @test
-- Children of (1, 2) are gone; the (3, 4) child survives.
SELECT id, a, b FROM cdc_child ORDER BY id;
