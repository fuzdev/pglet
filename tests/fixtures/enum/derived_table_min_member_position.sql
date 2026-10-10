-- A projected expression's **descriptor** and the order an outer clause sorts it by are
-- one fact, and they were two lists: the ordering walk had a `MIN`/`MAX` arm and
-- `expr_output_column` had none, so `min(m)` reached an enclosing `ORDER BY` with no
-- enum on its `ColumnDesc` and sorted by label bytes. `having_min_member_position` pins
-- the same aggregate one clause earlier — inside the aggregate node, where the walk
-- answers — and stayed green. Both derive from `expr_enum_with` now.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT g FROM (SELECT id AS g, min(m) AS mm FROM t GROUP BY id) q ORDER BY q.mm;
