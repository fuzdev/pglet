-- Three-column composite FOREIGN KEY with a NON-trivial permutation (not a simple
-- reversal): FOREIGN KEY (a, b, c) REFERENCES parent (z, x, y) against
-- PRIMARY KEY (x, y, z). The referenced list matches the PK as a SET; the local↔
-- parent pairing stays positional — a->z, b->x, c->y — so a child (a, b, c)
-- references parent (z=a, x=b, y=c). Exercises the general permute-into-index-order
-- probe beyond the 2-column full reversal.
-- @profile-lock
-- @setup
CREATE TABLE c3r_parent (x INT, y INT, z INT, PRIMARY KEY (x, y, z));
CREATE TABLE c3r_child (
  id INT PRIMARY KEY,
  a INT,
  b INT,
  c INT,
  FOREIGN KEY (a, b, c) REFERENCES c3r_parent (z, x, y)
);
INSERT INTO c3r_parent VALUES (1, 2, 3), (4, 5, 6);
-- For parent (x=1,y=2,z=3): a=z=3, b=x=1, c=y=2. For (4,5,6): a=6, b=4, c=5.
INSERT INTO c3r_child VALUES (10, 3, 1, 2), (11, 6, 4, 5);

-- @test
SELECT id, a, b, c FROM c3r_child ORDER BY id;
