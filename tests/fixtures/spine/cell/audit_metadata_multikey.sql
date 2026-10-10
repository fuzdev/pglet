-- The spine's audit-log lookup (fuz_app's `db/cell_audit_queries.ts`): the same id
-- is probed against
-- several metadata keys OR'd together — `metadata @> jsonb_build_object('cell_id',$1)
-- OR @> ('source_id',$1) OR @> ('parent_id',$1)` — over gin(metadata). Every disjunct
-- is a containment on the same column, so the OR-union routes all three probes to one
-- GinScan and unions their candidates; a recheck refines the lossy result. Proves the
-- union (a row matching ANY key is returned), not an intersection.
-- @profile-lock
-- @setup
CREATE TABLE audit_log (id int4, metadata jsonb);
CREATE INDEX idx_audit_metadata ON audit_log USING gin(metadata);
INSERT INTO audit_log (id, metadata) VALUES
  (1, '{"cell_id":"x1"}'),
  (2, '{"source_id":"x1"}'),
  (3, '{"parent_id":"x1"}'),
  (4, '{"cell_id":"x2"}'),
  (5, '{"other":"x1"}');
-- @param $1 text x1
-- @test
SELECT id FROM audit_log
WHERE metadata @> jsonb_build_object('cell_id', $1::text)
   OR metadata @> jsonb_build_object('source_id', $1::text)
   OR metadata @> jsonb_build_object('parent_id', $1::text)
ORDER BY id;
