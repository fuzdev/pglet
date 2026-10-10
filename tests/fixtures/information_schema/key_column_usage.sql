-- key_column_usage surfaces each FOREIGN KEY's referencing (child) column.
-- Scoped to FK rows via `position_in_unique_constraint IS NOT NULL` — both engines
-- leave it NULL for PK/UNIQUE, whose rows this fixture is not about (see
-- pk_key_column_usage.sql). pglet's FKs here are single-column, so
-- ordinal_position / position_in_unique_constraint are 1.
-- @setup
CREATE TABLE kcu_parent (id UUID PRIMARY KEY);
CREATE TABLE kcu_child (
  id UUID PRIMARY KEY,
  p UUID REFERENCES kcu_parent(id),
  q UUID REFERENCES kcu_parent(id)
);

-- @test
SELECT constraint_name::text AS constraint_name, table_name::text AS table_name,
       column_name::text AS column_name,
       ordinal_position::int AS ordinal_position,
       position_in_unique_constraint::int AS position_in_unique_constraint
FROM information_schema.key_column_usage
WHERE table_schema = 'public' AND table_name = 'kcu_child'
  AND position_in_unique_constraint IS NOT NULL
ORDER BY constraint_name;
