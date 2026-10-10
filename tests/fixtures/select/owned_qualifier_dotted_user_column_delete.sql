-- A qualifier a level owns binds a column of that relation, and a dot a user quoted into a
-- column's name is not a relation: `dq ("o.z", "o.n", n)` is one relation with three
-- ordinary columns, and under the alias `o`, `o.n` is the column `n` and `o."o.n"` the
-- column `"o.n"`. Whether the input is a join's rows is the plan's fact; a rule reading it
-- off a name that starts `o.` refused every owned reference over this table `42703`, and
-- the `"t.name"` spelling tier bound `o.n` to the user's `"o.n"` column.
--
-- This one: a `DELETE` filtered by a sub-select.
-- @setup
CREATE TABLE dq ("o.z" INT, "o.n" INT, n INT);
CREATE TABLE dq_t (y INT, n INT);
CREATE TABLE dq_w (k INT, n INT);
INSERT INTO dq VALUES (1, 50, 3), (2, 60, 4);
INSERT INTO dq_t VALUES (1, 10), (2, 20);
INSERT INTO dq_w VALUES (1, 100), (2, 200);

-- @test
DELETE FROM dq_w WHERE k IN (SELECT max(o.n) - 2 FROM dq o);
SELECT k, n FROM dq_w ORDER BY k;
