-- constraint_column_usage lists the columns a PRIMARY KEY / UNIQUE constraint uses.
-- Filtered to those two constraints by name so PG's CHECK / not-null rows for the
-- same table stay out.
-- @setup
CREATE TABLE isccu (id INT PRIMARY KEY, k INT UNIQUE, v INT);

-- @test
SELECT constraint_name::text AS constraint_name, table_name::text AS table_name,
       column_name::text AS column_name
FROM information_schema.constraint_column_usage
WHERE table_schema = 'public' AND constraint_name IN ('isccu_pkey', 'isccu_k_key')
ORDER BY constraint_name, column_name;
