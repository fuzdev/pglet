-- The spine's `query_session_enforce_limit` (fuz_app's `auth/session_queries.ts`) and
-- the same-shaped `query_api_token_enforce_limit`: keep the newest `$2` sessions for
-- an account and evict the rest, via a subquery-keyed DELETE bound over the extended
-- protocol —
--   DELETE FROM auth_session
--   WHERE id IN (SELECT id FROM auth_session WHERE account_id = $1
--                ORDER BY created_at DESC OFFSET $2) RETURNING id
-- created_at order differs from insertion order, and OFFSET keeps all but the
-- oldest, so exactly one deterministic row is returned — proving the eviction
-- selects by created_at (not insertion order) and that the IN-subquery plus the
-- bound OFFSET run end-to-end. (account_id is INT here for a self-contained fixture;
-- the spine's is UUID, and its inner ORDER BY adds an `id DESC` tiebreak these
-- distinct `created_at`s never reach — the query shape is what's under test.)
-- @setup
CREATE TABLE auth_session (id INT, account_id INT, created_at INT);
INSERT INTO auth_session VALUES
  (1, 7, 300),  -- newest    → kept
  (2, 7, 100),  -- oldest    → evicted
  (3, 7, 200),  -- middle    → kept
  (4, 8, 999);  -- other account, never touched
-- @param $1 int4 7
-- @param $2 int8 2
-- @test
DELETE FROM auth_session
WHERE id IN (
  SELECT id FROM auth_session
  WHERE account_id = $1
  ORDER BY created_at DESC
  OFFSET $2
) RETURNING id;
