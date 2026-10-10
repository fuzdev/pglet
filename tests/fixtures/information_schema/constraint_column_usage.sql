-- constraint_column_usage surfaces, for each FOREIGN KEY, the referenced
-- (parent) table + column. Filtered to FK constraints by the `_fkey` name suffix;
-- the PK/UNIQUE rows both engines also emit here are pk_constraint_column_usage.sql's
-- subject.
-- @setup
CREATE TABLE ccu_parent (id UUID PRIMARY KEY);
CREATE TABLE ccu_child (
  id UUID PRIMARY KEY,
  a UUID REFERENCES ccu_parent(id),
  b UUID REFERENCES ccu_parent(id)
);

-- @test
SELECT constraint_name::text AS constraint_name,
       table_name::text AS foreign_table_name,
       column_name::text AS foreign_column_name
FROM information_schema.constraint_column_usage
WHERE table_schema = 'public' AND constraint_name LIKE 'ccu_child%fkey'
ORDER BY constraint_name;
