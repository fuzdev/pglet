-- ...and reads *only* `pg_constraint`. A plain `CREATE INDEX canir_a_fkey` occupies the
-- relation namespace, which `ChooseConstraintName` never consults, so the foreign key
-- still takes `canir_a_fkey`. (A PK/UNIQUE auto-name is the one that must clear the
-- relation namespace too, because its index carries the name — a different function,
-- `ChooseRelationName`.)
-- @setup
CREATE TABLE canir_p (id INT PRIMARY KEY);
CREATE TABLE canir (a INT);
CREATE INDEX canir_a_fkey ON canir (a);
ALTER TABLE canir ADD FOREIGN KEY (a) REFERENCES canir_p(id);

-- @test
SELECT constraint_name::text AS constraint_name
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'canir' AND constraint_type = 'FOREIGN KEY'
ORDER BY constraint_name;
