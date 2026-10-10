-- The cell-authz correlated EXISTS (fuz_app's `db/cell_queries.ts`, `query_cell_list`): a
-- viewer sees a cell iff a grant row exists for it. The inner subquery correlates
-- on the OUTER `c.id` while the inner `cell_grant g` ALSO has its own `id` column,
-- so the qualified `c.id` is only resolvable with alias-aware scoping (qualifier
-- `c` ≠ the inner alias `g` → outer; `g.cell_id`/`g.grantee` → inner). Grants are
-- filtered to the bound grantee, mirroring the spine's per-viewer authz check.
-- @setup
CREATE TABLE cell (
  id    UUID PRIMARY KEY,
  data  JSONB NOT NULL
);
CREATE TABLE cell_grant (
  id        UUID PRIMARY KEY,
  cell_id   UUID NOT NULL,
  grantee   UUID NOT NULL
);
INSERT INTO cell (id, data) VALUES
  ('11111111-1111-1111-1111-111111111111', '{"kind":"doc"}'),
  ('22222222-2222-2222-2222-222222222222', '{"kind":"note"}'),
  ('33333333-3333-3333-3333-333333333333', '{"kind":"secret"}');
-- The viewer (grantee aaaa…) is granted cells 1 and 2 but not 3.
INSERT INTO cell_grant (id, cell_id, grantee) VALUES
  ('aaaaaaa1-0000-0000-0000-000000000000', '11111111-1111-1111-1111-111111111111', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa'),
  ('aaaaaaa2-0000-0000-0000-000000000000', '22222222-2222-2222-2222-222222222222', 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa'),
  ('bbbbbbb3-0000-0000-0000-000000000000', '33333333-3333-3333-3333-333333333333', 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb');
-- @param $1 uuid aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa
-- @test
SELECT c.id
  FROM cell c
  WHERE EXISTS (
    SELECT 1 FROM cell_grant g WHERE g.cell_id = c.id AND g.grantee = $1
  )
  ORDER BY c.id;
