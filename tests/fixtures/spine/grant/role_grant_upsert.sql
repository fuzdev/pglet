-- Mirrors, reduced, the spine's `role_grant` / `role_grant_offer` upserts (fuz_app's
-- `auth/role_grant_queries.ts` / `auth/role_grant_offer_queries.ts`, whose keys
-- also coalesce the scope id to a sentinel): here one active grant per (actor_id,
-- role, COALESCE(scope_kind, '')), enforced by an
-- **expression partial** unique index (`… WHERE revoked_at IS NULL`), maintained
-- via `INSERT … ON CONFLICT (actor_id, role, COALESCE(scope_kind, '')) WHERE
-- revoked_at IS NULL DO UPDATE …`. The expression/partial `ON CONFLICT` *target*
-- is the ~101-failure matrix gap pglet's own parser was built to close; it now
-- parses natively and runs differentially against real Postgres like any fixture.
-- @setup
CREATE TABLE role_grant (
  actor_id   INT NOT NULL,
  role       TEXT NOT NULL,
  scope_kind TEXT,
  note       TEXT,
  revoked_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX role_grant_active ON role_grant (actor_id, role, COALESCE(scope_kind, '')) WHERE revoked_at IS NULL;
INSERT INTO role_grant (actor_id, role, scope_kind, note) VALUES (1, 'admin', NULL, 'first');
-- @test
INSERT INTO role_grant (actor_id, role, scope_kind, note) VALUES (1, 'admin', NULL, 'second')
  ON CONFLICT (actor_id, role, COALESCE(scope_kind, '')) WHERE revoked_at IS NULL
  DO UPDATE SET note = EXCLUDED.note;
INSERT INTO role_grant (actor_id, role, scope_kind, note) VALUES (1, 'admin', 'repo', 'scoped')
  ON CONFLICT (actor_id, role, COALESCE(scope_kind, '')) WHERE revoked_at IS NULL
  DO UPDATE SET note = EXCLUDED.note;
SELECT actor_id, role, scope_kind, note FROM role_grant ORDER BY note;
