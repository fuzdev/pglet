-- ON DELETE SET NULL on an ALTER-added FK column nulls the referencing children
-- when their parent is deleted (mirrors the spine's auth v1 `source_offer_id`
-- column added via ALTER TABLE ... ADD COLUMN ... REFERENCES ... ON DELETE SET NULL).
-- @profile-lock
-- @setup
CREATE TABLE fk_parent (id INT PRIMARY KEY, name TEXT);
CREATE TABLE fk_child (id INT PRIMARY KEY);
ALTER TABLE fk_child ADD COLUMN parent_id INT REFERENCES fk_parent(id) ON DELETE SET NULL;
INSERT INTO fk_parent VALUES (1, 'a'), (2, 'b');
INSERT INTO fk_child VALUES (10, 1), (11, 1), (20, 2);
DELETE FROM fk_parent WHERE id = 1;

-- @test
-- Children of parent 1 survive with parent_id nulled; parent 2's child unchanged.
SELECT id, parent_id FROM fk_child ORDER BY id;
