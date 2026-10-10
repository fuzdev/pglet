-- A child whose ALTER-added FK column matches an existing parent (and a NULL FK
-- value, which a FK always permits) inserts cleanly.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY);
ALTER TABLE fk_child ADD COLUMN parent_id INT REFERENCES fk_parent(id);
INSERT INTO fk_parent VALUES (1, 'a'), (2, 'b');
INSERT INTO fk_child VALUES (10, 1), (11, 2), (12, NULL);

-- @test
SELECT id, parent_id FROM fk_child ORDER BY id;
