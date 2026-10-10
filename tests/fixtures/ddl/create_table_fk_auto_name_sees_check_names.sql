-- Inside one `CREATE TABLE` the FK is added after the CHECKs, so its auto-name sees
-- theirs: a CHECK declared `ctfk_a_fkey` pushes the table-level FOREIGN KEY to
-- `ctfk_a_fkey1`. One claimed-name set spanning the whole statement is what makes that
-- work; per-kind pools let every cross-kind pair through.
-- @setup
CREATE TABLE ctfk_p (id INT PRIMARY KEY);
CREATE TABLE ctfk (a INT CONSTRAINT ctfk_a_fkey CHECK (a < 9),
                   FOREIGN KEY (a) REFERENCES ctfk_p(id));

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'ctfk'
ORDER BY constraint_name;
