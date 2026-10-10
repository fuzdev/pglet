-- ON DELETE CASCADE over a UUID foreign key. Deleting ONE parent must cascade
-- to ONLY that parent's children — the other parent's children survive.
--
-- A child-matcher whose comparison folds every UUID pair to `Equal` matches EVERY
-- child to the deleted parent and wipes the whole table, whichever parent is deleted;
-- the other parent's surviving children are what catch it.
-- @setup
CREATE TABLE uuid_parent (id UUID PRIMARY KEY, name TEXT);
CREATE TABLE uuid_child (
  id UUID PRIMARY KEY,
  parent_id UUID REFERENCES uuid_parent(id) ON DELETE CASCADE,
  label TEXT
);
INSERT INTO uuid_parent VALUES
  ('11111111-1111-1111-1111-111111111111', 'p1'),
  ('22222222-2222-2222-2222-222222222222', 'p2');
INSERT INTO uuid_child VALUES
  ('aaaaaaaa-0000-0000-0000-000000000001', '11111111-1111-1111-1111-111111111111', 'c1a'),
  ('aaaaaaaa-0000-0000-0000-000000000002', '11111111-1111-1111-1111-111111111111', 'c1b'),
  ('bbbbbbbb-0000-0000-0000-000000000001', '22222222-2222-2222-2222-222222222222', 'c2a');
-- Delete only parent 1: its two children cascade away; parent 2's child remains.
DELETE FROM uuid_parent WHERE id = '11111111-1111-1111-1111-111111111111';
-- @test
SELECT label, parent_id FROM uuid_child ORDER BY label;
