-- ON UPDATE SET NULL onto a NOT NULL child column: nulling would violate the child's
-- NOT NULL, so the whole UPDATE aborts (23502).
-- @profile-lock
-- @setup
CREATE TABLE ousnn_parent (id INT PRIMARY KEY);
CREATE TABLE ousnn_child (
  id INT PRIMARY KEY,
  parent_id INT NOT NULL REFERENCES ousnn_parent(id) ON UPDATE SET NULL
);
INSERT INTO ousnn_parent VALUES (1);
INSERT INTO ousnn_child VALUES (10, 1);

-- @test
-- @expect-error: SET NULL would violate the child's NOT NULL (23502)
UPDATE ousnn_parent SET id = 2 WHERE id = 1;
