-- A child whose FK value matches an existing parent (and a NULL FK value, which a
-- FK always permits) inserts cleanly.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY, parent_id INT REFERENCES fk_parent(id));
INSERT INTO fk_parent VALUES (1, 'a'), (2, 'b');
INSERT INTO fk_child VALUES (10, 1), (11, 2), (12, NULL);

-- @test
SELECT id, parent_id FROM fk_child ORDER BY id;
