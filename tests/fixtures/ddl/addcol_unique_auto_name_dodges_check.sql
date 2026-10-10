-- An auto PK/UNIQUE index name also clears the schema's constraint names
-- (`ChooseRelationName`'s `isconstraint` pass finishes with `ConstraintNameExists`), so
-- an existing CHECK called `auand_c_key` pushes the added column's UNIQUE to
-- `auand_c_key1`. This only became reachable once unnamed CHECKs carried stored names —
-- before, the CHECK was invisible to the namer.
-- @setup
CREATE TABLE auand (a INT, CONSTRAINT auand_c_key CHECK (a > 0));
ALTER TABLE auand ADD COLUMN c INT UNIQUE;

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'auand'
ORDER BY constraint_name;
