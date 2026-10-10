-- query_cell_delete (fuz_app's `db/cell_queries.ts`): soft-delete a cell, stamping
-- `deleted_at` / `updated_at` / `updated_by` and returning `id` so the
-- caller knows whether an active row was tombstoned. `$2` is the
-- (nullable) deleter actor.
-- @profile-lock
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE actor (id UUID PRIMARY KEY DEFAULT gen_random_uuid());
INSERT INTO actor (id) VALUES ('aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa');
CREATE TABLE cell (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  data        JSONB NOT NULL,
  visibility  cell_visibility NOT NULL DEFAULT 'private',
  path        TEXT,
  refs        TEXT[],
  created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at  TIMESTAMPTZ,
  deleted_at  TIMESTAMPTZ,
  created_by  UUID REFERENCES actor(id) ON DELETE SET NULL,
  updated_by  UUID REFERENCES actor(id) ON DELETE SET NULL
);
INSERT INTO cell (id, data) VALUES
  ('11111111-1111-1111-1111-111111111111', '{"kind":"doc"}');
-- @param $1 uuid 11111111-1111-1111-1111-111111111111
-- @param $2 uuid aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa
-- @test
UPDATE cell SET deleted_at = NOW(), updated_at = NOW(), updated_by = $2
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING id;
