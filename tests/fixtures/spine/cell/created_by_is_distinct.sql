-- Isolates the `c.created_by IS DISTINCT FROM $N` clause from the spine's
-- `cell_list` read (fuz_app's `db/cell_queries.ts`): a null-safe author filter over
-- a nullable uuid column, with the parameter typed from its column sibling (no cast).
-- Rows cover all three cases: created_by NULL (distinct from any uuid → kept),
-- equal to $1 (not distinct → dropped), and a different uuid (distinct → kept).
-- (The full cell_list query also needs correlated EXISTS, which is out of scope;
-- only the IS DISTINCT FROM clause is lifted here.)
-- @setup
CREATE TABLE cell (
  id          UUID PRIMARY KEY,
  created_by  UUID
);
INSERT INTO cell (id, created_by) VALUES
  ('11111111-1111-1111-1111-111111111111', NULL),
  ('22222222-2222-2222-2222-222222222222', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa'),
  ('33333333-3333-3333-3333-333333333333', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb');
-- @param $1 uuid aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa
-- @test
SELECT id FROM cell WHERE created_by IS DISTINCT FROM $1 ORDER BY id;
