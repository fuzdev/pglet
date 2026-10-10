-- information_schema.referential_constraints.update_rule reflects the FK's ON UPDATE
-- action (CASCADE / SET NULL / RESTRICT / NO ACTION), alongside delete_rule.
-- @no-declared-types: same information_schema `name`/`varchar`-vs-`text` gap as
--   `information_schema/tables_projection.sql` — pglet's catalog columns are declared
--   `text`, and the `varchar` half is unaffected by `name` existing as a type.
-- @setup
CREATE TABLE ir_parent (id INT PRIMARY KEY);
CREATE TABLE ir_child (
  id INT PRIMARY KEY,
  p1 INT REFERENCES ir_parent(id) ON UPDATE CASCADE ON DELETE SET NULL
);

-- @test
SELECT constraint_name, update_rule, delete_rule
FROM information_schema.referential_constraints
WHERE constraint_name = 'ir_child_p1_fkey';
