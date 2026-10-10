-- The two namespaces are separate in the *other* direction too, so the gate must not
-- over-reject. Two shapes both engines accept, and **both setup halves are load-bearing
-- assertions** — don't trim either one when editing:
--
-- 1. a CHECK named after an index that backs no constraint (that index is only in the
--    relation namespace, so widening the constraint check to "any index on this table"
--    would break it); and
-- 2. `cnpi2` — the same declared constraint name on a *different table*, which is the
--    only place the corpus pins the **declared** gate's per-relation scope
--    (`pg_constraint`'s unique index is keyed `(conrelid, conname)`;
--    `ConstraintNameIsUsed`, "deliberately not the same test"). Its counterpart, the
--    schema-wide **auto** scope, is `constraint_auto_name_scope_is_schema_wide.sql`.
--    Widening the declared gate to the auto scope reddens only this fixture — nothing
--    else in the corpus rejects the two-table shape — so losing `cnpi2` would leave a
--    change that bricks ordinary hand-written DDL entirely green.
-- @setup
CREATE TABLE cnpi (a INT);
CREATE INDEX cnpi_plain ON cnpi (a);
ALTER TABLE cnpi ADD CONSTRAINT cnpi_plain CHECK (a > 0);
CREATE TABLE cnpi2 (a INT, CONSTRAINT cnpi_plain CHECK (a > 0));

-- @test
SELECT table_name::text AS table_name, constraint_name::text AS constraint_name,
       constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name LIKE 'cnpi%' AND constraint_type = 'CHECK'
ORDER BY table_name, constraint_name;
