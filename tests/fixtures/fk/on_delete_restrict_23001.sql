-- ON DELETE RESTRICT: a referencing child rejects the parent delete with Postgres's
-- distinct restrict_violation SQLSTATE (23001), separate from NO ACTION's 23503.
-- @profile-lock
-- @setup
CREATE TABLE odr_parent (id INT PRIMARY KEY);
CREATE TABLE odr_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES odr_parent(id) ON DELETE RESTRICT
);
INSERT INTO odr_parent VALUES (1);
INSERT INTO odr_child VALUES (10, 1);

-- @test
-- @expect-error: RESTRICT rejects deleting a referenced parent (23001)
DELETE FROM odr_parent WHERE id = 1;
