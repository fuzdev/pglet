-- The same disambiguation inside one `CREATE TABLE`: an inline `REFERENCES` and a
-- table-level `FOREIGN KEY` over the same column both auto-name `fkct_a_fkey`, so the
-- second takes `fkct_a_fkey1`. The names a statement has already handed out are visible
-- to its next constraint (Postgres's `CommandCounterIncrement`), which is why the
-- second one has to look past the first rather than fail.
-- @setup
CREATE TABLE fkct_p (id INT PRIMARY KEY);
CREATE TABLE fkct (a INT REFERENCES fkct_p(id), FOREIGN KEY (a) REFERENCES fkct_p(id));

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'fkct' AND constraint_type = 'FOREIGN KEY'
ORDER BY constraint_name;
