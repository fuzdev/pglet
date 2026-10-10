-- A stored expression's array cast renders its modifier **inside** the `[]`, because
-- the modifier is the element's. `pg_get_constraintdef` and
-- `information_schema.check_constraints.check_clause` are two readers of the one
-- `deparse` Catalog rendering, so both are asserted here.
--
-- Rendering it after the whole name emitted `character varying[](3)`, which is not a
-- value 18.6 ever prints and does not re-parse on either engine.
-- @setup
CREATE TABLE tmcd (a varchar(3)[], b numeric(4,1)[],
  CONSTRAINT tmcd_a_check CHECK (a <> '{zzz}'::varchar(3)[]),
  CONSTRAINT tmcd_b_check CHECK (b <> '{9.9}'::numeric(4,1)[]));
-- @no-declared-types: 18.6 sends `name` (19) for `conname` / `constraint_name` and
--   `information_schema.character_data` for `check_clause`, where pglet sends `text`
--   (25) — pre-existing catalog-type modelling, not this fixture's subject.
-- @test
SELECT c.conname, pg_get_constraintdef(c.oid) AS def, k.check_clause
FROM pg_constraint c
JOIN information_schema.check_constraints k ON k.constraint_name = c.conname
WHERE c.conname LIKE 'tmcd\_%'
ORDER BY c.conname;
