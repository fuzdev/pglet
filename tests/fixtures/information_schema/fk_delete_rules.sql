-- fuz_app's `db` suite's FK-delete-rule introspection: a 4-way join across
-- table_constraints / key_column_usage / referential_constraints /
-- constraint_column_usage, surfacing each FK's (table, column) -> (parent table,
-- parent column) with its delete_rule (CASCADE vs SET NULL).
-- @setup
CREATE TABLE fkr_parent (id UUID PRIMARY KEY);
CREATE TABLE fkr_child (
  id UUID PRIMARY KEY,
  parent_id UUID NOT NULL REFERENCES fkr_parent(id) ON DELETE CASCADE,
  owner_id UUID REFERENCES fkr_parent(id) ON DELETE SET NULL
);

-- @test
SELECT tc.table_name::text AS table_name, kcu.column_name::text AS column_name,
       ccu.table_name::text AS foreign_table_name,
       ccu.column_name::text AS foreign_column_name,
       rc.delete_rule::text AS delete_rule
  FROM information_schema.table_constraints tc
  JOIN information_schema.key_column_usage kcu
    ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema
  JOIN information_schema.referential_constraints rc
    ON tc.constraint_name = rc.constraint_name
  JOIN information_schema.constraint_column_usage ccu
    ON ccu.constraint_name = tc.constraint_name
 WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public'
   AND tc.table_name LIKE 'fkr_%'
 ORDER BY tc.table_name, kcu.column_name;
