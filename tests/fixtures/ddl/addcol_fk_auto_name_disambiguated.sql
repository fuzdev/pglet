-- `ALTER TABLE … ADD COLUMN … REFERENCES` mints its FK name through the same gate as
-- every other route. It used to claim nothing at all, so its auto-name could land on
-- top of an existing constraint of that name — here a CHECK — leaving two rows sharing
-- one `(table_name, constraint_name)`. That pair is unique by construction in
-- `pg_constraint`, so an introspecting client has no way to see the collision; Postgres
-- gives the FK `afad_c_fkey1`.
-- @setup
CREATE TABLE afad_p (id INT PRIMARY KEY);
CREATE TABLE afad (a INT, CONSTRAINT afad_c_fkey CHECK (a > 0));
ALTER TABLE afad ADD COLUMN c INT REFERENCES afad_p(id);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'afad'
ORDER BY constraint_name;
