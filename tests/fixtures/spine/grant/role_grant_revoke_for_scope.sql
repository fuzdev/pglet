-- The spine's `query_role_grant_revoke_for_scope` (fuz_app's
-- `auth/role_grant_queries.ts`, its first statement) — a data-modifying CTE bound
-- over the extended protocol: revoke every active grant at a scope, then
-- join the revoked rows to `actor` to resolve each `account_id` for fan-out, in one
-- round-trip. `$1` (scope_id) types from the UPDATE's `WHERE`; `$2`/`$3` from the
-- `SET` assignments. `revoked_at = NOW()` is set but excluded from the output, so the
-- compared projection is deterministic.
-- @setup
CREATE TABLE actor (id INT, account_id INT);
INSERT INTO actor VALUES (100, 9001), (101, 9002);
CREATE TABLE role_grant (
  id INT, role TEXT, scope_kind TEXT, scope_id INT, actor_id INT,
  revoked_at TIMESTAMPTZ, revoked_by INT, revoked_reason TEXT
);
INSERT INTO role_grant VALUES
  (1, 'admin',  'repo', 5, 100, NULL, NULL, NULL),
  (2, 'writer', 'repo', 5, 101, NULL, NULL, NULL),
  (3, 'reader', 'org',  7, 100, NULL, NULL, NULL);
-- @param $1 int4 5
-- @param $2 int4 9999
-- @param $3 text scope-deleted
-- @test
WITH updated AS (
  UPDATE role_grant
  SET revoked_at = NOW(), revoked_by = $2, revoked_reason = $3
  WHERE scope_id = $1 AND revoked_at IS NULL
  RETURNING id, role, scope_kind, scope_id, actor_id
)
SELECT u.id AS role_grant_id, u.role, u.scope_kind, u.scope_id, u.actor_id, a.account_id
FROM updated u
JOIN actor a ON a.id = u.actor_id
ORDER BY u.id;
