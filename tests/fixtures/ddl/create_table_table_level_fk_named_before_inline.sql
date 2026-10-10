-- A `CREATE TABLE`'s elements are named in **written** order, across the column-level
-- and table-level spellings alike — Postgres walks `stmt->tableElts` once, routing both
-- kinds onto the same `cxt.fkconstraints` list, so the table-level `FOREIGN KEY` written
-- first takes the base auto-name and the later inline `REFERENCES` takes the digit.
--
-- The order is invisible to a name-only query: both `roftl_a_fkey` and `roftl_a_fkey1`
-- exist either way. It becomes observable by giving the two FKs *different* referential
-- actions and reading which name carries which — folding the column-level constraints
-- ahead of the table-level ones pairs `roftl_a_fkey` with SET NULL instead of CASCADE,
-- a persisted name attached to the wrong specification.
-- @setup
CREATE TABLE roftl_p (id INT PRIMARY KEY);
CREATE TABLE roftl (FOREIGN KEY (a) REFERENCES roftl_p (id) ON DELETE CASCADE,
                    a INT REFERENCES roftl_p (id) ON DELETE SET NULL);

-- @test
SELECT constraint_name::text AS constraint_name, delete_rule::text AS delete_rule
FROM information_schema.referential_constraints
WHERE constraint_name LIKE 'roftl\_a\_fkey%'
ORDER BY constraint_name;
