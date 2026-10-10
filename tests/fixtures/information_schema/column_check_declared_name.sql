-- A column-level `CONSTRAINT <n> CHECK (…)` keeps its name. The parser used to drop
-- it, which was not inert: the constraint was invisible in `table_constraints` under
-- any name, and — since it held no name — the same name could be reused on the same
-- table without collision, which is 42710 in Postgres.
-- @setup
CREATE TABLE cccn (a INT CONSTRAINT cccn_positive CHECK (a > 0), b INT);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'cccn' AND constraint_type = 'CHECK'
ORDER BY constraint_name;
