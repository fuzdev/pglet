-- The schema-wide auto-name scan spans all four kinds, because `pg_constraint` holds
-- all four: a `UNIQUE` named `cansk_b_a_check` on another table pushes `cansk_b`'s
-- auto-named CHECK to `cansk_b_a_check1`. Reading only the CHECK/FK half would leave
-- the PK/UNIQUE names invisible to the namer.
-- @setup
CREATE TABLE cansk_a (a INT, CONSTRAINT cansk_b_a_check UNIQUE (a));
CREATE TABLE cansk_b (a INT, CHECK (a > 0));

-- @test
SELECT table_name::text AS table_name, constraint_name::text AS constraint_name,
       constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name IN ('cansk_a', 'cansk_b')
ORDER BY table_name, constraint_name;
