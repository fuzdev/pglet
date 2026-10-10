-- An **auto** constraint name is disambiguated against the whole *schema*, not just the
-- table it lands on: `ChooseConstraintName` scans `pg_constraint (conname,
-- connamespace)`. So a CHECK named `cansw_b_a_check` sitting on `cansw_a` pushes
-- `cansw_b`'s own auto-named CHECK to `cansw_b_a_check1`, even though nothing on
-- `cansw_b` holds that name.
--
-- The scope is the opposite of the **declared**-name gate, which is per-relation
-- (`ConstraintNameIsUsed` — "deliberately not the same test"). Getting the pair
-- backwards changes which names collide with no other symptom, which is why it is
-- pinned here rather than left to the naming fixtures above.
-- @setup
CREATE TABLE cansw_a (a INT);
CREATE TABLE cansw_b (a INT);
ALTER TABLE cansw_a ADD CONSTRAINT cansw_b_a_check CHECK (a > 0);
ALTER TABLE cansw_b ADD CHECK (a > 0);

-- @test
SELECT table_name::text AS table_name, constraint_name::text AS constraint_name
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name IN ('cansw_a', 'cansw_b')
  AND constraint_type = 'CHECK'
ORDER BY table_name, constraint_name;
