-- The real scoped-role-grant shape: a partial composite unique index mixing bare
-- columns with COALESCE expressions —
--   (actor_id, role, COALESCE(scope_kind,'GLOBAL'), COALESCE(scope_id, <sentinel>))
--   WHERE revoked_at IS NULL.
-- Two rows with the same (actor_id, role) and NULL scope both fold to the
-- GLOBAL/sentinel key, so the second collides. (A row with an explicit distinct
-- scope is allowed — see expr_unique_coalesce_distinct_scope_allowed.)
-- @profile-lock
-- @setup
CREATE TABLE expr_coalesce_grant (
  id INT,
  actor_id UUID,
  role TEXT,
  scope_kind TEXT,
  scope_id UUID,
  revoked_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX ux_expr_coalesce_grant ON expr_coalesce_grant (
  actor_id,
  role,
  COALESCE(scope_kind, 'GLOBAL'),
  COALESCE(scope_id, '00000000-0000-0000-0000-000000000000'::uuid)
) WHERE revoked_at IS NULL;
INSERT INTO expr_coalesce_grant VALUES
  (1, '11111111-1111-1111-1111-111111111111', 'admin', NULL, NULL, NULL);

-- @test
-- @expect-error: a second all-NULL-scope grant folds to the same GLOBAL key
INSERT INTO expr_coalesce_grant VALUES
  (2, '11111111-1111-1111-1111-111111111111', 'admin', NULL, NULL, NULL);
