-- A multi-row parent UPDATE cascades each row's key change to its own children (no
-- transient unique collision: 1->101, 2->102).
-- @profile-lock
-- @setup
CREATE TABLE oumr_parent (id INT PRIMARY KEY);
CREATE TABLE oumr_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES oumr_parent(id) ON UPDATE CASCADE
);
INSERT INTO oumr_parent VALUES (1), (2);
INSERT INTO oumr_child VALUES (10, 1), (20, 2);
UPDATE oumr_parent SET id = id + 100;

-- @test
SELECT id, parent_id FROM oumr_child ORDER BY id;
