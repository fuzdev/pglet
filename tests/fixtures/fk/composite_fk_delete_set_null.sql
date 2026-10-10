-- Composite FOREIGN KEY with ON DELETE SET NULL: deleting a parent nulls EVERY
-- referencing column (a and b) of each matching child; the children survive.
-- @profile-lock
-- @setup
CREATE TABLE cds_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE cds_child (
  id INT PRIMARY KEY,
  a INT,
  b INT,
  FOREIGN KEY (a, b) REFERENCES cds_parent (x, y) ON DELETE SET NULL
);
INSERT INTO cds_parent VALUES (1, 2), (3, 4);
INSERT INTO cds_child VALUES (10, 1, 2), (11, 1, 2), (20, 3, 4);
DELETE FROM cds_parent WHERE x = 1 AND y = 2;

-- @test
-- Children of (1, 2) survive with BOTH a and b nulled; the (3, 4) child unchanged.
SELECT id, a, b FROM cds_child ORDER BY id;
