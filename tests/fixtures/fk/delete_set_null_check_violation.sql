-- ON DELETE SET NULL that would violate a CHECK must abort the DELETE. The FK
-- column is nullable (so the plain NOT-NULL guard doesn't fire), but a CHECK
-- requires it non-null. Nulling it on cascade leaves the row violating the CHECK,
-- so Postgres rejects the DELETE and the parent stays. pglet must do the same
-- (the SET-NULL'd image is re-validated against CHECK before any write).
-- @profile-lock
-- @setup
CREATE TABLE snc_parent (id INT PRIMARY KEY);
CREATE TABLE snc_child (
  id INT PRIMARY KEY,
  parent_id INT REFERENCES snc_parent(id) ON DELETE SET NULL,
  CHECK (parent_id IS NOT NULL)
);
INSERT INTO snc_parent VALUES (1);
INSERT INTO snc_child VALUES (10, 1);
-- @test
-- @expect-error: SET NULL would leave the CHECK (parent_id IS NOT NULL) violated
DELETE FROM snc_parent WHERE id = 1;
