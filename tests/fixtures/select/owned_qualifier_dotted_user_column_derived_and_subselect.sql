-- A qualifier a level owns binds a column of that relation, and a dot a user quoted into a
-- column's name is not a relation: `dq ("o.z", "o.n", n)` is one relation with three
-- ordinary columns, and under the alias `o`, `o.n` is the column `n` and `o."o.n"` the
-- column `"o.n"`. Whether the input is a join's rows is the plan's fact; a rule reading it
-- off a name that starts `o.` refused every owned reference over this table `42703`, and
-- the `"t.name"` spelling tier bound `o.n` to the user's `"o.n"` column.
--
-- This one: a derived table whose output carries a dotted name, and a sub-select under an enclosing `o`.
-- @setup
CREATE TABLE dq ("o.z" INT, "o.n" INT, n INT);
CREATE TABLE dq_t (y INT, n INT);
CREATE TABLE dq_w (k INT, n INT);
INSERT INTO dq VALUES (1, 50, 3), (2, 60, 4);
INSERT INTO dq_t VALUES (1, 10), (2, 20);
INSERT INTO dq_w VALUES (1, 100), (2, 200);

-- @test
SELECT y,
       (SELECT max(s.n) FROM (SELECT 1 AS "s.z", 2 AS n) s) AS derived_n,
       (SELECT max(s."s.z") FROM (SELECT 1 AS "s.z", 2 AS n) s) AS derived_z,
       (SELECT max(o.n) FROM dq o) AS sub_n,
       (SELECT o.n FROM dq o ORDER BY o.n DESC LIMIT 1) AS sub_top
FROM dq_t o ORDER BY y;
