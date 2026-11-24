-- A batched first-child probe: one `CROSS JOIN LATERAL (subquery)` per parent id of a
-- bound `uuid[]`, the subquery joining a link table to its nodes, filtering on the
-- parent's id, a `jsonb` path, a soft delete and a visibility predicate over bound
-- parameters and a `WITH` query, ordered by position and cut to one row. A parent with
-- no viewable child is absent, and the parents come back in the array's order.
-- @setup
CREATE TABLE lnode (
  id uuid PRIMARY KEY,
  kind text NOT NULL,
  data jsonb NOT NULL,
  deleted_at timestamptz,
  owner_id uuid
);
CREATE TABLE llink (
  parent_id uuid NOT NULL,
  child_id uuid NOT NULL,
  position int NOT NULL,
  PRIMARY KEY (parent_id, child_id)
);
CREATE TABLE lgrant (actor_id uuid, role text);
INSERT INTO lnode VALUES
  ('00000000-0000-0000-0000-0000000000a1', 'folder', '{}', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000a2', 'folder', '{}', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000a3', 'folder', '{}', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000b1', 'entry', '{"meta":{"thumb":"t1"}}', NULL, '00000000-0000-0000-0000-0000000000c1'),
  ('00000000-0000-0000-0000-0000000000b2', 'entry', '{"meta":{"thumb":"t2"}}', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000b3', 'entry', '{"meta":{}}', NULL, NULL),
  ('00000000-0000-0000-0000-0000000000b4', 'entry', '{"meta":{"thumb":"t4"}}', '2024-01-01', NULL),
  ('00000000-0000-0000-0000-0000000000b5', 'note', '{"meta":{"thumb":"t5"}}', NULL, NULL);
INSERT INTO llink VALUES
  ('00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-0000000000b3', 0),
  ('00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-0000000000b2', 2),
  ('00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-0000000000b1', 1),
  ('00000000-0000-0000-0000-0000000000a2', '00000000-0000-0000-0000-0000000000b4', 0),
  ('00000000-0000-0000-0000-0000000000a2', '00000000-0000-0000-0000-0000000000b5', 1),
  ('00000000-0000-0000-0000-0000000000a2', '00000000-0000-0000-0000-0000000000b2', 3);
INSERT INTO lgrant VALUES ('00000000-0000-0000-0000-0000000000c1', 'viewer');
-- @param $1 uuid[] {00000000-0000-0000-0000-0000000000a2,00000000-0000-0000-0000-0000000000a1,00000000-0000-0000-0000-0000000000a3}
-- @param $2 bool false
-- @param $3 uuid 00000000-0000-0000-0000-0000000000c1
-- @param $4 text[] {viewer}
-- @test
WITH caller_roles AS (SELECT role FROM lgrant WHERE actor_id = $3 AND role = ANY($4))
SELECT parent.id AS parent_id, cover.thumb, cover.node_id
FROM unnest($1::uuid[]) AS parent(id)
CROSS JOIN LATERAL (
  SELECT child.data#>>'{meta,thumb}' AS thumb, child.id AS node_id
  FROM llink li
  JOIN lnode child ON child.id = li.child_id
  WHERE li.parent_id = parent.id
    AND child.deleted_at IS NULL
    AND child.kind = 'entry'
    AND child.data#>>'{meta,thumb}' IS NOT NULL
    AND ($2 OR child.owner_id = $3 OR EXISTS (SELECT 1 FROM caller_roles))
  ORDER BY li.position ASC
  LIMIT 1
) cover
WHERE cover.thumb IS NOT NULL;
