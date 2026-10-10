-- table_constraints carries PRIMARY KEY and UNIQUE rows, named by Postgres's
-- <table>_pkey / <table>_<cols>_key convention. A standalone CREATE UNIQUE INDEX is
-- deliberately absent (it is an index, not a constraint) — so the row count below is
-- three, not four. Filtered to PK/UNIQUE so PG 18's <col>_not_null CHECK rows don't
-- intrude.
-- @setup
CREATE TABLE ispk (id INT PRIMARY KEY, k INT UNIQUE, v INT);
CREATE TABLE ispk2 (a INT, b INT, PRIMARY KEY (a, b));
CREATE UNIQUE INDEX ispk_v_ux ON ispk (v);

-- @test
SELECT constraint_name::text AS constraint_name, table_name::text AS table_name,
       constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name IN ('ispk', 'ispk2')
  AND constraint_type IN ('PRIMARY KEY', 'UNIQUE')
ORDER BY constraint_name;
