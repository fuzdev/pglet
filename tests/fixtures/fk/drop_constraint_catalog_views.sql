-- After the drop, the constraint is gone from every catalog surface that listed it:
-- `pg_constraint`, `information_schema.table_constraints` and
-- `information_schema.referential_constraints`.
-- @setup
CREATE TABLE dccv_p (id INT PRIMARY KEY);
CREATE TABLE dccv_c (a INT REFERENCES dccv_p(id), b INT REFERENCES dccv_p(id));

-- @test
ALTER TABLE dccv_c DROP CONSTRAINT dccv_c_a_fkey;
SELECT 'pg_constraint' AS surface, conname::text AS name FROM pg_constraint
WHERE conname LIKE 'dccv%'
UNION ALL
SELECT 'table_constraints', constraint_name::text FROM information_schema.table_constraints
WHERE table_name = 'dccv_c'
UNION ALL
SELECT 'referential_constraints', constraint_name::text
FROM information_schema.referential_constraints WHERE constraint_name::text LIKE 'dccv%'
ORDER BY surface, name;
