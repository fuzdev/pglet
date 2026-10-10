-- Companion to delete_cascade_two_level: verifies the MIDDLE level. After
-- deleting grandparent 1 (all-CASCADE chain gp -> p -> c), the parent rows under
-- gp 1 (p 10, 11) are cascade-deleted; only p 20 (under gp 2) survives.
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
SELECT id, gp_id FROM fk_p ORDER BY id;
