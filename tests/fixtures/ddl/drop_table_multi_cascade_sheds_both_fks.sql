-- The position-drift guard, and the ONLY shape that sees it.
--
-- `cext` carries two foreign keys, at positions 0 and 1, one to each target. The
-- resolver identifies an FK by `(table, position)` and removes positions high-to-low
-- *within one* `Dependents`. Calling that once per target computes both positions
-- against the same pre-mutation catalog, so dropping `ca` shortens `cext`'s vector and
-- the second call's position 1 no longer exists — `if i < child.foreign_keys.len()`
-- silently no-ops and `cext_y_fkey` survives, referencing a table that is gone.
--
-- The INSERT is what makes that observable rather than merely untidy: pglet's
-- `enforce_foreign_keys` treats a missing parent tuple as a violation, so a surviving
-- constraint makes every later write to `cext` fail `23503` naming a vanished table.
-- Reaching the SELECT at all is therefore half the assertion.
-- @setup
CREATE TABLE ca (id INT PRIMARY KEY);
CREATE TABLE cb (id INT PRIMARY KEY);
CREATE TABLE cext (x INT REFERENCES ca, y INT REFERENCES cb);

-- @test
DROP TABLE ca, cb CASCADE;
INSERT INTO cext VALUES (99, 99);
SELECT count(*)::int AS surviving_fks FROM information_schema.table_constraints
WHERE table_name = 'cext' AND constraint_type = 'FOREIGN KEY';
