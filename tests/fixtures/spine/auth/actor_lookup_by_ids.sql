-- The spine's `query_actors_by_ids` (fuz_app's `auth/actor_lookup_queries.ts`): resolve a
-- batch of actors to their account username + display name over the actor→account
-- JOIN, keyed by a BARE (uncast) uuid-array param —
--   SELECT act.id, a.username, act.name AS display_name
--   FROM actor act JOIN account a ON a.id = act.account_id
--   WHERE act.id = ANY($1)
-- The `$1` element type is inferred from the qualified `act.id` across the join's
-- FROM scope (alias `act` → table `actor`); without that the bare array param stays
-- untyped and `ANY` fails loud. (ORDER BY added for a deterministic comparison — the
-- spine leaves order unspecified.)
-- @setup
CREATE TABLE account (id uuid PRIMARY KEY, username TEXT NOT NULL);
CREATE TABLE actor (id uuid PRIMARY KEY, account_id uuid NOT NULL, name TEXT NOT NULL);
INSERT INTO account (id, username) VALUES
  ('00000000-0000-0000-0000-0000000000a1', 'alice'),
  ('00000000-0000-0000-0000-0000000000b2', 'bob');
INSERT INTO actor (id, account_id, name) VALUES
  ('00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-0000000000a1', 'Alice A'),
  ('00000000-0000-0000-0000-000000000022', '00000000-0000-0000-0000-0000000000b2', 'Bob B'),
  ('00000000-0000-0000-0000-000000000033', '00000000-0000-0000-0000-0000000000a1', 'Alice Alt');
-- @param $1 uuid[] {00000000-0000-0000-0000-000000000011,00000000-0000-0000-0000-000000000033}
-- @test
SELECT act.id, a.username, act.name AS display_name
FROM actor act
JOIN account a ON a.id = act.account_id
WHERE act.id = ANY($1)
ORDER BY display_name;
