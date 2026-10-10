-- Composite FOREIGN KEY whose referenced list is reordered relative to the parent's
-- PRIMARY KEY: `REFERENCES parent (y, x)` against `PRIMARY KEY (x, y)` is covered
-- (the referenced columns match the unique key as a SET). The local↔parent pairing
-- stays positional — a->y, b->x — so a child (a, b) references parent (y=a, x=b).
-- @profile-lock
-- @setup
CREATE TABLE crc_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE crc_child (
  id INT PRIMARY KEY,
  a INT,
  b INT,
  FOREIGN KEY (a, b) REFERENCES crc_parent (y, x)
);
INSERT INTO crc_parent VALUES (1, 2), (3, 4);
-- a=2,b=1 -> parent (y=2, x=1) = (x=1, y=2) EXISTS; a=4,b=3 -> (x=3, y=4) EXISTS.
INSERT INTO crc_child VALUES (10, 2, 1), (11, 4, 3);

-- @test
SELECT id, a, b FROM crc_child ORDER BY id;
