-- `information_schema.table_constraints` reads the one name the catalog holds: the cut
-- one, for each kind.
-- @setup
CREATE TABLE cac9_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT UNIQUE CHECK (cooooooooooooooooooooooooooooooooooooooo > 0) REFERENCES cac9_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt, id INT PRIMARY KEY);
-- @test
SELECT constraint_type::text AS constraint_type, constraint_name::text AS constraint_name
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'cac9_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt' AND constraint_name NOT LIKE '%not\_null'
ORDER BY constraint_type, constraint_name;
