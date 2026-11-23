-- The same migration run a second time, over tables whose foreign keys are already
-- gone: every `IF EXISTS` name is the skip, and the statement succeeds.
-- @setup
CREATE TABLE actor (id UUID PRIMARY KEY, updated_by UUID, deleted_by UUID);
CREATE TABLE role_grant (id UUID PRIMARY KEY, granted_by UUID, revoked_by UUID,
  source_offer_id UUID);

-- @test
ALTER TABLE role_grant DROP CONSTRAINT IF EXISTS role_grant_granted_by_fkey, DROP CONSTRAINT IF EXISTS role_grant_revoked_by_fkey, DROP CONSTRAINT IF EXISTS role_grant_source_offer_id_fkey;
ALTER TABLE actor DROP CONSTRAINT IF EXISTS actor_updated_by_fkey, DROP CONSTRAINT IF EXISTS actor_deleted_by_fkey;
SELECT conname, contype FROM pg_constraint
WHERE conname LIKE 'actor%' OR conname LIKE 'role_grant%' ORDER BY conname;
