-- Default (NO ACTION) FK: updating a parent's referenced key while a child still
-- points at the old value orphans the child and is rejected (23503).
-- @profile-lock
-- @setup
CREATE TABLE oup_parent (id INT PRIMARY KEY);
CREATE TABLE oup_child (id INT PRIMARY KEY, parent_id INT REFERENCES oup_parent(id));
INSERT INTO oup_parent VALUES (1);
INSERT INTO oup_child VALUES (10, 1);

-- @test
-- @expect-error: NO ACTION rejects orphaning the child (23503)
UPDATE oup_parent SET id = 2 WHERE id = 1;
