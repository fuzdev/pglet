-- A seed migration's shape: `INSERT … SELECT <constants> WHERE EXISTS (…) ON CONFLICT … DO
-- NOTHING`, a select with no FROM whose `WHERE` decides whether its one row is written.
-- One referenced row exists and one does not, so one statement writes its row and the
-- other writes none; the bare `'a'` takes the column's type as an unknown literal does.
-- @setup
CREATE TABLE node (id uuid PRIMARY KEY, label text);
CREATE TABLE node_link (
  parent_id uuid NOT NULL REFERENCES node(id),
  slot text NOT NULL,
  child_id uuid NOT NULL REFERENCES node(id),
  PRIMARY KEY (parent_id, slot)
);
INSERT INTO node (id, label) VALUES
  ('00000000-0000-4000-8000-000000000001', 'parent'),
  ('00000000-0000-4000-8000-000000000002', 'child');
INSERT INTO node_link (parent_id, slot, child_id)
SELECT '00000000-0000-4000-8000-000000000001'::uuid, 'a', '00000000-0000-4000-8000-000000000002'::uuid
WHERE EXISTS (SELECT 1 FROM node WHERE id = '00000000-0000-4000-8000-000000000002'::uuid)
ON CONFLICT (parent_id, slot) DO NOTHING;
INSERT INTO node_link (parent_id, slot, child_id)
SELECT '00000000-0000-4000-8000-000000000003'::uuid, 'b', '00000000-0000-4000-8000-000000000001'::uuid
WHERE EXISTS (SELECT 1 FROM node WHERE id = '00000000-0000-4000-8000-000000000003'::uuid)
ON CONFLICT (parent_id, slot) DO NOTHING;
-- @test
SELECT parent_id, slot, child_id FROM node_link ORDER BY slot;
