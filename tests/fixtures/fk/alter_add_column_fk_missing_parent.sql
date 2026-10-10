-- An FK declared on an ALTER-added column is enforced: inserting a child whose
-- ALTER-added FK value has no matching parent violates the FK.
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY);
ALTER TABLE fk_child ADD COLUMN parent_id INT REFERENCES fk_parent(id);
INSERT INTO fk_parent VALUES (1, 'a');

-- @test
-- @expect-error: insert references a missing parent via the ALTER-added FK
INSERT INTO fk_child VALUES (10, 2);
