-- An FK's **auto**-name is disambiguated, never gated. Postgres calls
-- `ChooseConstraintName` only when no `CONSTRAINT <n>` was written, and it appends a
-- digit until the name is free — so issuing the same unnamed `ADD FOREIGN KEY` twice
-- succeeds as `fkad_a_fkey` + `fkad_a_fkey1`. pglet baked the auto-name at parse time,
-- which made declared-vs-auto structurally indistinguishable and put both through the
-- declared-name gate: the second statement was 42710.
-- @setup
CREATE TABLE fkad_p (id INT PRIMARY KEY);
CREATE TABLE fkad (a INT);
ALTER TABLE fkad ADD FOREIGN KEY (a) REFERENCES fkad_p(id);
ALTER TABLE fkad ADD FOREIGN KEY (a) REFERENCES fkad_p(id);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'fkad' AND constraint_type = 'FOREIGN KEY'
ORDER BY constraint_name;
