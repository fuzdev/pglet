-- A 9-param variant of fuz_app's `query_cell_update` (`db/cell_queries.ts`, which
-- binds 10, gating `updated_by` behind its own flag): a partial cell update with
-- params buried inside CASE expressions in the SET clause (the
-- present-or-keep pattern), plus a cast to the `cell_visibility` enum. Exercises
-- parameter-type inference + ParameterDescription over the extended protocol the
-- way tokio-postgres drives it — a strict count check
-- node-postgres tolerates. $6/$9 are typed NULLs (path/visibility not provided).
-- @profile-lock
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (
  id uuid PRIMARY KEY,
  data jsonb,
  refs text[],
  path text,
  visibility cell_visibility NOT NULL DEFAULT 'private',
  updated_by uuid,
  updated_at timestamptz,
  deleted_at timestamptz
);
INSERT INTO cell (id, data, path, visibility)
  VALUES ('11111111-1111-1111-1111-111111111111', '{"n": 1}', 'notes/a', 'private');
-- @param $1 uuid 11111111-1111-1111-1111-111111111111
-- @param $2 bool true
-- @param $3 text {"n": 2, "ref": "blake3:bbbb"}
-- @param $4 text[] {blake3:bbbb}
-- @param $5 bool false
-- @param $6 text NULL
-- @param $7 uuid 22222222-2222-2222-2222-222222222222
-- @param $8 bool false
-- @param $9 text NULL
-- @test
UPDATE cell SET
  data       = CASE WHEN $2::bool THEN $3::text::jsonb ELSE data END,
  refs       = CASE WHEN $2::bool THEN $4::text[] ELSE refs END,
  path       = CASE WHEN $5::bool THEN $6 ELSE path END,
  updated_by = $7,
  visibility = CASE WHEN $8::bool THEN $9::text::cell_visibility ELSE visibility END,
  updated_at = NOW()
WHERE id = $1 AND deleted_at IS NULL
RETURNING id, path, refs, data;
