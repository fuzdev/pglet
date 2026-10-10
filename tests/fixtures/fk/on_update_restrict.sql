-- ON UPDATE RESTRICT: a referencing child rejects the parent-key update with
-- Postgres's distinct restrict_violation SQLSTATE (23001, not 23503).
-- @profile-lock
-- @setup
CREATE TABLE our_parent (id INT PRIMARY KEY);
CREATE TABLE our_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES our_parent(id) ON UPDATE RESTRICT
);
INSERT INTO our_parent VALUES (1);
INSERT INTO our_child VALUES (10, 1);

-- @test
-- @expect-error: RESTRICT rejects the parent-key update (23001)
UPDATE our_parent SET id = 2 WHERE id = 1;
