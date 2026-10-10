-- @setup
CREATE TABLE trunc_parent (id INT PRIMARY KEY);
CREATE TABLE trunc_child (id INT, parent_id INT REFERENCES trunc_parent(id));
INSERT INTO trunc_parent VALUES (1);
INSERT INTO trunc_child VALUES (10, 1);
TRUNCATE trunc_parent CASCADE;

-- @test
SELECT
  (SELECT count(*) FROM trunc_parent) AS parent,
  (SELECT count(*) FROM trunc_child) AS child;
