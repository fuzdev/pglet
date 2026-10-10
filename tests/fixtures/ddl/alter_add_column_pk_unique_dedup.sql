-- `ADD COLUMN` shares the merge: an added column declared both PRIMARY KEY and UNIQUE
-- yields one index, and it is the primary key's.
-- @setup
CREATE TABLE aacu (x INT);
ALTER TABLE aacu ADD COLUMN c INT PRIMARY KEY UNIQUE;

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'aacu'
  AND constraint_type IN ('PRIMARY KEY', 'UNIQUE')
ORDER BY constraint_name;
