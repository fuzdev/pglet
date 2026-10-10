-- `information_schema.check_constraints.check_clause` is the second reader of the same
-- deparser, and it moves with `pg_get_constraintdef` — one rendering, two surfaces.
-- @setup
CREATE TABLE ckc (level TEXT, s TEXT);
ALTER TABLE ckc ADD CONSTRAINT ckc_level CHECK (level IN ('viewer', 'editor'));
ALTER TABLE ckc ADD CONSTRAINT ckc_s CHECK (s = 'x');
-- @test
SELECT constraint_name::text AS constraint_name, check_clause::text AS check_clause
FROM information_schema.check_constraints
WHERE constraint_name::text IN ('ckc_level', 'ckc_s')
ORDER BY constraint_name::text ASC
