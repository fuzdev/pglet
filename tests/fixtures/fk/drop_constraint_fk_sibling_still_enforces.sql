-- Dropping one of two foreign keys from the same child leaves the other enforcing: the
-- parent delete the survivor blocks is still `23503`, naming the survivor.
-- @expect-error: the surviving foreign key still blocks the delete
-- @expect-message
-- @profile-lock
-- @setup
CREATE TABLE dcfs_p (id INT PRIMARY KEY);
CREATE TABLE dcfs_c (
  a INT REFERENCES dcfs_p(id),
  b INT,
  CONSTRAINT dcfs_keep FOREIGN KEY (b) REFERENCES dcfs_p(id)
);
INSERT INTO dcfs_p VALUES (1);
INSERT INTO dcfs_c VALUES (1, 1);

-- @test
ALTER TABLE dcfs_c DROP CONSTRAINT dcfs_c_a_fkey;
DELETE FROM dcfs_p WHERE id = 1;
