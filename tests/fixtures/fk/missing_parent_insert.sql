-- Inserting a child whose FK value has no matching parent violates the FK.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY, parent_id INT REFERENCES fk_parent(id));
INSERT INTO fk_parent VALUES (1, 'a');

-- @test
-- @expect-error: insert references a missing parent
INSERT INTO fk_child VALUES (10, 2);
