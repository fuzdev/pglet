-- CHECK + FOREIGN KEY rows match Postgres (name + type). Filtered to the two
-- explicitly-named constraints (so PK / NOT NULL noise and PG's domain types
-- don't intrude), columns cast to text.
-- @setup
CREATE TABLE isc_parent (id INT PRIMARY KEY);
CREATE TABLE isc_child (
  id INT,
  parent_id INT,
  CONSTRAINT isc_child_fk FOREIGN KEY (parent_id) REFERENCES isc_parent(id),
  CONSTRAINT isc_child_ck CHECK (id > 0)
);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND constraint_name IN ('isc_child_fk', 'isc_child_ck')
ORDER BY constraint_name;
