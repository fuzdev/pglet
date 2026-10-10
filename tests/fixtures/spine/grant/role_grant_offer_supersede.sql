-- The pending-offer supersede inside fuz_app's `query_revoke_role_grant`
-- (`auth/role_grant_queries.ts`) — a data-modifying `UPDATE … FROM … RETURNING` CTE
-- (fuz_app returns named columns, this fixture `o.*`) bound over the extended
-- protocol. It stamps `superseded_at` on every pending offer to the `$1` actor's
-- account `(role, scope)` (the actor whose grant was revoked), joining `actor` to
-- resolve the account from the actor id, then re-joins the superseded rows to `actor`
-- for the grantor's account_id.
-- `$1` (actor id) types from `a.id`, `$2` (role) from `o.role`, `$3` (scope_id)
-- from `o.scope_id`. `superseded_at = NOW()` is non-deterministic, so the compared
-- projection takes only its presence (`IS NOT NULL`), not its value.
-- @setup
CREATE TABLE actor (id INT, account_id INT);
INSERT INTO actor VALUES (100, 9001), (101, 9002), (200, 7000);
CREATE TABLE role_grant_offer (
  id INT, to_account_id INT, from_actor_id INT, role TEXT, scope_id INT,
  accepted_at TIMESTAMPTZ, declined_at TIMESTAMPTZ, retracted_at TIMESTAMPTZ,
  superseded_at TIMESTAMPTZ
);
INSERT INTO role_grant_offer VALUES
  (1, 9001, 200, 'admin',  5, NULL, NULL, NULL, NULL),
  (2, 9001, 200, 'admin',  5, '2020-01-01T00:00:00Z', NULL, NULL, NULL),
  (3, 9001, 200, 'writer', 5, NULL, NULL, NULL, NULL),
  (4, 9002, 200, 'admin',  5, NULL, NULL, NULL, NULL);
-- @param $1 int4 100
-- @param $2 text admin
-- @param $3 int4 5
-- @test
WITH updated AS (
  UPDATE role_grant_offer o
  SET superseded_at = NOW()
  FROM actor a
  WHERE a.id = $1
    AND o.to_account_id = a.account_id
    AND o.role = $2
    AND o.scope_id IS NOT DISTINCT FROM $3
    AND o.accepted_at IS NULL
    AND o.declined_at IS NULL
    AND o.retracted_at IS NULL
    AND o.superseded_at IS NULL
  RETURNING o.*
)
SELECT u.id, u.role, u.scope_id, u.to_account_id,
       (u.superseded_at IS NOT NULL) AS was_superseded,
       grantor.account_id AS from_account_id
FROM updated u
JOIN actor grantor ON grantor.id = u.from_actor_id
ORDER BY u.id;
