-- The migration runner's detect_old_tracker probe: scalar EXISTS over
-- information_schema.columns. Output is a deterministic bool pair (sidesteps
-- Postgres's wide, domain-typed information_schema.columns projection).
-- @setup
CREATE TABLE schema_version (namespace TEXT, name TEXT, sequence INTEGER, applied_at TIMESTAMPTZ);

-- @test
SELECT
  EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'schema_version' AND column_name = 'namespace') AS has_namespace,
  EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'schema_version' AND column_name = 'version') AS has_version;
