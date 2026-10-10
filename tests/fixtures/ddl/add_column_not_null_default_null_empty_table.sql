-- The control against over-rejecting the statement above: over an **empty** table the
-- same `ADD COLUMN … NOT NULL DEFAULT NULL` succeeds on 18.6 (measured — the column
-- lands with `attnotnull` set, `atthasdef` false and no `attmissingval`), because there
-- are no rows for the rewrite to fail on. A gate keyed on the *default* rather than on
-- the backfilled values would refuse this.
--
-- The `@test` reads the resulting catalog rather than any row, so it also asserts what
-- 18.6 records: the column is NOT NULL and carries **no** default.
-- @setup
CREATE TABLE ancd_e (id INT);
ALTER TABLE ancd_e ADD COLUMN c INT NOT NULL DEFAULT NULL;
-- @test
SELECT column_name::text AS column_name,
       is_nullable::text AS is_nullable,
       column_default::text AS column_default,
       (column_default IS NULL) AS default_is_sql_null
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'ancd_e'
ORDER BY ordinal_position;
