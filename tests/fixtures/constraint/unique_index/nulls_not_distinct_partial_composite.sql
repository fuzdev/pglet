-- The real spine `idx_cell_grant_unique_role_scope` shape: a composite, PARTIAL,
-- `NULLS NOT DISTINCT` unique index over (cell_id, role, scope_id) WHERE role IS
-- NOT NULL. Two grants sharing (cell_id, role) with a NULL scope_id collide,
-- because NULLS NOT DISTINCT makes the two NULL scope_ids equal — and both pass
-- the partial predicate (role is not null).
-- @profile-lock
-- @setup
CREATE TABLE cell_grant (cell_id INT, role TEXT, scope_id INT);
CREATE UNIQUE INDEX idx_cell_grant_unique_role_scope
  ON cell_grant (cell_id, role, scope_id) NULLS NOT DISTINCT WHERE role IS NOT NULL;
INSERT INTO cell_grant VALUES (1, 'editor', NULL);

-- @test
-- @expect-error: a second (1,'editor',NULL) collides under NULLS NOT DISTINCT
INSERT INTO cell_grant VALUES (1, 'editor', NULL);
