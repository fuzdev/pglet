-- The auth migration's idempotent-ADD-CONSTRAINT look-before-leap: probe
-- information_schema.table_constraints for a named CHECK (added via ALTER), then
-- the absent case. Deterministic count pair (filters by name).
-- @setup
CREATE TABLE isc_rg (scope_kind TEXT, scope_id UUID);
ALTER TABLE isc_rg ADD CONSTRAINT isc_rg_pair CHECK ((scope_kind IS NULL) = (scope_id IS NULL));

-- @test
SELECT
  (SELECT count(*) FROM information_schema.table_constraints WHERE table_name = 'isc_rg' AND constraint_name = 'isc_rg_pair') AS has_pair,
  (SELECT count(*) FROM information_schema.table_constraints WHERE table_name = 'isc_rg' AND constraint_name = 'absent') AS has_absent;
