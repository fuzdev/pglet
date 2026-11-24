-- A one-row `FOR NO KEY UPDATE SKIP LOCKED` sweep probe over a bound key and an
-- interval comparison: with no other connection holding the writer it answers the row.
-- @profile-lock
-- @setup
CREATE TABLE lskipx (id uuid PRIMARY KEY, owner_id uuid, expires_at timestamptz);
INSERT INTO lskipx VALUES
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-0000000000aa', '2020-01-01T00:00:00Z'),
  ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-0000000000bb', NULL);
-- @param $1 uuid 00000000-0000-0000-0000-000000000001
-- @param $2 int8 60
-- @test
SELECT owner_id FROM lskipx
WHERE id = $1
  AND expires_at IS NOT NULL
  AND expires_at + $2::bigint * INTERVAL '1 second' <= NOW()
FOR NO KEY UPDATE SKIP LOCKED;
