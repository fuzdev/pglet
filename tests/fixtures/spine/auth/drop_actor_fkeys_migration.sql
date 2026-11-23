-- The fuz_auth migration that drops the actor-referencing foreign keys by their default
-- names: one `ALTER TABLE` per table, each a comma list of `DROP CONSTRAINT IF EXISTS`.
-- Afterwards the constraints are gone from `pg_constraint`, a row naming no actor writes,
-- and deleting an actor nulls no column that referenced it.
-- @profile-lock
-- @setup
CREATE TABLE actor (
  id UUID PRIMARY KEY,
  updated_by UUID REFERENCES actor(id) ON DELETE SET NULL,
  deleted_by UUID REFERENCES actor(id) ON DELETE SET NULL
);
CREATE TABLE role_grant_offer (id UUID PRIMARY KEY);
CREATE TABLE role_grant (
  id UUID PRIMARY KEY,
  granted_by UUID REFERENCES actor(id) ON DELETE SET NULL,
  revoked_by UUID REFERENCES actor(id) ON DELETE SET NULL,
  source_offer_id UUID REFERENCES role_grant_offer(id) ON DELETE SET NULL
);
INSERT INTO actor VALUES ('00000000-0000-0000-0000-000000000001', NULL, NULL);
INSERT INTO actor VALUES ('00000000-0000-0000-0000-000000000002',
  '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001');
INSERT INTO role_grant_offer VALUES ('00000000-0000-0000-0000-0000000000a1');
INSERT INTO role_grant VALUES ('00000000-0000-0000-0000-0000000000b1',
  '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001',
  '00000000-0000-0000-0000-0000000000a1');

-- @test
ALTER TABLE role_grant DROP CONSTRAINT IF EXISTS role_grant_granted_by_fkey, DROP CONSTRAINT IF EXISTS role_grant_revoked_by_fkey, DROP CONSTRAINT IF EXISTS role_grant_source_offer_id_fkey;
ALTER TABLE actor DROP CONSTRAINT IF EXISTS actor_updated_by_fkey, DROP CONSTRAINT IF EXISTS actor_deleted_by_fkey;
INSERT INTO role_grant VALUES ('00000000-0000-0000-0000-0000000000b2',
  '00000000-0000-0000-0000-0000000000ff', NULL, '00000000-0000-0000-0000-0000000000fe');
DELETE FROM actor WHERE id = '00000000-0000-0000-0000-000000000001';
DELETE FROM role_grant_offer;
SELECT 'constraint' AS kind, conname::text || ':' || contype::text AS v FROM pg_constraint
WHERE conname LIKE 'actor%' OR conname LIKE 'role_grant%'
UNION ALL
SELECT 'actor', id::text || '/' || coalesce(updated_by::text, '-') || '/'
  || coalesce(deleted_by::text, '-') FROM actor
UNION ALL
SELECT 'grant', id::text || '/' || coalesce(granted_by::text, '-') || '/'
  || coalesce(source_offer_id::text, '-') FROM role_grant
ORDER BY kind, v;
