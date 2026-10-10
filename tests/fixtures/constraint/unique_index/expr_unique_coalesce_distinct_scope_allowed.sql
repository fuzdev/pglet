-- The COALESCE scoped-grant index allows a row with an explicit distinct scope
-- alongside the all-NULL (GLOBAL/sentinel) row: the explicit scope_kind/scope_id
-- fold to a different key, so both coexist.
-- @setup
CREATE TABLE expr_coalesce_distinct (
  id INT,
  actor_id UUID,
  role TEXT,
  scope_kind TEXT,
  scope_id UUID,
  revoked_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX ux_expr_coalesce_distinct ON expr_coalesce_distinct (
  actor_id,
  role,
  COALESCE(scope_kind, 'GLOBAL'),
  COALESCE(scope_id, '00000000-0000-0000-0000-000000000000'::uuid)
) WHERE revoked_at IS NULL;
INSERT INTO expr_coalesce_distinct VALUES
  (1, '11111111-1111-1111-1111-111111111111', 'admin', NULL, NULL, NULL),
  (2, '11111111-1111-1111-1111-111111111111', 'admin', 'team',
      '22222222-2222-2222-2222-222222222222', NULL);

-- @test
SELECT id, role, scope_kind FROM expr_coalesce_distinct ORDER BY id;
