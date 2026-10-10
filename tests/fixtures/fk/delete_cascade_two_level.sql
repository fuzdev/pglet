-- Multi-level ON DELETE CASCADE: grandparent -> parent -> child, all CASCADE.
-- Deleting the grandparent must remove the entire chain with no orphans left at
-- any level. The @test reads the deepest table (the leaf children): if cascade
-- only went one level, c 100/101 would orphan; full cascade leaves only c 200.
-- @profile-lock
-- @setup
CREATE TABLE fk_gp (id INT PRIMARY KEY);
CREATE TABLE fk_p (
  id INT PRIMARY KEY,
  gp_id INT REFERENCES fk_gp(id) ON DELETE CASCADE
);
CREATE TABLE fk_c (
  id INT PRIMARY KEY,
  p_id INT REFERENCES fk_p(id) ON DELETE CASCADE
);
INSERT INTO fk_gp VALUES (1), (2);
INSERT INTO fk_p VALUES (10, 1), (11, 1), (20, 2);
INSERT INTO fk_c VALUES (100, 10), (101, 11), (200, 20);
DELETE FROM fk_gp WHERE id = 1;

-- @test
-- Only the leaf child under gp 2's subtree (c 200) survives; c 100 and c 101
-- (two levels below the deleted gp 1) are cascade-deleted, proving multi-level.
SELECT id, p_id FROM fk_c ORDER BY id;
