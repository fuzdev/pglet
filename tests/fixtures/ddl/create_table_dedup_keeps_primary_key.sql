-- The merge is not symmetric. The primary key is kept in preference to any UNIQUE it
-- absorbs, so `a INT PRIMARY KEY UNIQUE` reports ONE constraint and it is a PRIMARY
-- KEY — and a *named* UNIQUE merged into an unnamed PK transfers its name onto the
-- survivor ("if we have named and unnamed constraints, we'll use (at least one of)
-- the names for the index"), so the primary key here is called `ctdk2_myu`.
-- `ctdk3` writes the UNIQUE *first*, which is what separates the two rules: the
-- primary key is hoisted to the head of the merge list before the walk, so it absorbs
-- an earlier UNIQUE rather than being absorbed by it, and still takes that UNIQUE's
-- name.
-- @setup
CREATE TABLE ctdk1 (a INT PRIMARY KEY UNIQUE);
CREATE TABLE ctdk2 (a INT PRIMARY KEY, CONSTRAINT ctdk2_myu UNIQUE (a));
CREATE TABLE ctdk3 (a INT NOT NULL, CONSTRAINT ctdk3_myu UNIQUE (a), PRIMARY KEY (a));

-- @test
SELECT table_name::text AS table_name, constraint_name::text AS constraint_name,
       constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name LIKE 'ctdk%'
  AND constraint_type IN ('PRIMARY KEY', 'UNIQUE')
ORDER BY table_name, constraint_name;
