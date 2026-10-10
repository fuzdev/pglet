-- The spine's scoped `query_actor_search` (fuz_app's `auth/actor_search_queries.ts`):
-- prefix-search actors by lowercased name, restricted to those holding an *active*
-- role_grant on one of the supplied scopes —
--   SELECT DISTINCT act.id, a.username, act.name AS display_name
--   FROM actor act
--   JOIN account a ON a.id = act.account_id
--   JOIN role_grant rg ON rg.actor_id = act.id
--     AND rg.scope_id = ANY($2)
--     AND rg.revoked_at IS NULL
--     AND (rg.expires_at IS NULL OR rg.expires_at > NOW())
--   WHERE LOWER(act.name) LIKE $1 || '%' ESCAPE '\'
--   ORDER BY display_name, id
--   LIMIT $3
-- The bare uuid-array `$2` lives in the *join ON* clause under `SELECT DISTINCT`;
-- its element type is inferred from the alias-qualified `rg.scope_id` via the
-- FROM-scope alias map, which must see through the `Plan::Distinct` node. Without
-- that `$2` stays untyped, decodes as text, and `ANY` fails — the join silently
-- matches 0 rows. This exercises: scoped active-role filtering (revoked/expired
-- excluded), the DISTINCT multi-grant collapse, the scope filter, and LIMIT.
-- @setup
CREATE TABLE account (id uuid PRIMARY KEY, username TEXT NOT NULL);
CREATE TABLE actor (id uuid PRIMARY KEY, account_id uuid NOT NULL, name TEXT NOT NULL);
CREATE TABLE role_grant (
  id uuid PRIMARY KEY,
  actor_id uuid NOT NULL,
  scope_id uuid,
  revoked_at timestamptz,
  expires_at timestamptz
);
INSERT INTO account (id, username) VALUES
  ('00000000-0000-0000-0000-0000000000a1', 'alice'),
  ('00000000-0000-0000-0000-0000000000a2', 'bob'),
  ('00000000-0000-0000-0000-0000000000a3', 'carol');
INSERT INTO actor (id, account_id, name) VALUES
  ('00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-0000000000a1', 'Alice'),
  ('00000000-0000-0000-0000-000000000022', '00000000-0000-0000-0000-0000000000a2', 'Anna'),
  ('00000000-0000-0000-0000-000000000033', '00000000-0000-0000-0000-0000000000a3', 'Andrew'),
  ('00000000-0000-0000-0000-000000000044', '00000000-0000-0000-0000-0000000000a1', 'Alex'),
  ('00000000-0000-0000-0000-000000000055', '00000000-0000-0000-0000-0000000000a2', 'Bob'),
  ('00000000-0000-0000-0000-000000000066', '00000000-0000-0000-0000-0000000000a3', 'Amy');
-- scope C0 is the searched scope; C9 is a different scope (Amy holds a grant there).
-- Alice holds TWO active grants on C0 → DISTINCT must collapse them to one row.
-- Andrew's grant is revoked; Alex's is expired; both are excluded. Bob's name
-- fails the 'a%' prefix. Amy's active grant is on the wrong scope.
INSERT INTO role_grant (id, actor_id, scope_id, revoked_at, expires_at) VALUES
  ('00000000-0000-0000-0000-0000000000f1', '00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-0000000000c0', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000f2', '00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-0000000000c0', NULL, '2999-01-01T00:00:00Z'),
  ('00000000-0000-0000-0000-0000000000f3', '00000000-0000-0000-0000-000000000022', '00000000-0000-0000-0000-0000000000c0', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000f4', '00000000-0000-0000-0000-000000000033', '00000000-0000-0000-0000-0000000000c0', '2020-01-01T00:00:00Z', NULL),
  ('00000000-0000-0000-0000-0000000000f5', '00000000-0000-0000-0000-000000000044', '00000000-0000-0000-0000-0000000000c0', NULL, '2000-01-01T00:00:00Z'),
  ('00000000-0000-0000-0000-0000000000f6', '00000000-0000-0000-0000-000000000055', '00000000-0000-0000-0000-0000000000c0', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000f7', '00000000-0000-0000-0000-000000000066', '00000000-0000-0000-0000-0000000000c9', NULL, NULL);
-- @param $1 text a
-- @param $2 uuid[] {00000000-0000-0000-0000-0000000000c0}
-- @param $3 int8 10
-- @test
SELECT DISTINCT act.id, a.username, act.name AS display_name
FROM actor act
JOIN account a ON a.id = act.account_id
JOIN role_grant rg ON rg.actor_id = act.id
  AND rg.scope_id = ANY($2)
  AND rg.revoked_at IS NULL
  AND (rg.expires_at IS NULL OR rg.expires_at > NOW())
WHERE LOWER(act.name) LIKE $1 || '%' ESCAPE '\'
ORDER BY display_name, id
LIMIT $3;
