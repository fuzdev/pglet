-- A PARTIAL unique index (`WHERE deleted_at IS NULL`) is enforced over the rows
-- the predicate selects: two ACTIVE rows (deleted_at NULL) with the same path
-- collide, so the second insert violates the constraint.
-- @profile-lock
-- @setup
CREATE TABLE partial_active (id INT, path TEXT, deleted_at TIMESTAMPTZ);
CREATE UNIQUE INDEX ux_partial_active ON partial_active (path) WHERE deleted_at IS NULL;
INSERT INTO partial_active VALUES (1, '/a', NULL);

-- @test
-- @expect-error: duplicate active path violates the partial unique index
INSERT INTO partial_active VALUES (2, '/a', NULL);
