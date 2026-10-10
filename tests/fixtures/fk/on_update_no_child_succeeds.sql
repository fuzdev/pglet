-- A parent-key UPDATE with no referencing child succeeds (nothing to orphan).
-- @profile-lock
-- @setup
CREATE TABLE ounc_parent (id INT PRIMARY KEY);
CREATE TABLE ounc_child (id INT PRIMARY KEY, parent_id INT REFERENCES ounc_parent(id));
INSERT INTO ounc_parent VALUES (1);
UPDATE ounc_parent SET id = 2 WHERE id = 1;

-- @test
SELECT id FROM ounc_parent ORDER BY id;
