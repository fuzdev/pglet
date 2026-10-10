-- ON DELETE SET NULL that collides on a NULLS NOT DISTINCT unique index must abort
-- the DELETE. Each child references a different parent (so the unique on parent_id
-- is satisfied at insert: 1 and 2 are distinct). Deleting BOTH parents nulls BOTH
-- children's parent_id — and under NULLS NOT DISTINCT two NULL keys collide, so
-- Postgres rejects the DELETE. pglet must too: the cascade's SET-NULL'd images are
-- re-validated for uniqueness, including a collision between two siblings the same
-- cascade nulls (which a storage probe alone can't see — neither new image is
-- written yet).
-- @profile-lock
-- @setup
CREATE TABLE nndc_parent (id INT PRIMARY KEY);
CREATE TABLE nndc_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES nndc_parent(id) ON DELETE SET NULL
);
CREATE UNIQUE INDEX nndc_child_pid ON nndc_child (parent_id) NULLS NOT DISTINCT;
INSERT INTO nndc_parent VALUES (1), (2);
INSERT INTO nndc_child VALUES (10, 1), (20, 2);
-- @test
-- @expect-error: SET NULL nulls both children, colliding under NULLS NOT DISTINCT
DELETE FROM nndc_parent WHERE id IN (1, 2);
