-- Updating a non-referenced column, or a no-op `SET id = id`, never triggers the
-- parent-side FK action even with a live child.
-- @setup
CREATE TABLE ounk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE ounk_child (id INT PRIMARY KEY, parent_id INT REFERENCES ounk_parent(id));
INSERT INTO ounk_parent VALUES (1, 'a');
INSERT INTO ounk_child VALUES (10, 1);
UPDATE ounk_parent SET name = 'b' WHERE id = 1;
UPDATE ounk_parent SET id = id WHERE id = 1;

-- @test
SELECT p.id, p.name, c.parent_id
FROM ounk_parent p JOIN ounk_child c ON c.parent_id = p.id
ORDER BY p.id;
