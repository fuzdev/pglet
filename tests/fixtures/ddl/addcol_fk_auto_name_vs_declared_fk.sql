-- The same unclaimed-name hole between two foreign keys: a *declared*
-- `afvd_b_fkey` and then an `ADD COLUMN b … REFERENCES` whose auto-name is also
-- `afvd_b_fkey`. Two same-named FKs on one table are not merely cosmetic — the
-- DROP-time dependency resolver identifies a foreign key by `(table, position)`
-- precisely because a name-keyed removal would shed the wrong sibling.
-- @setup
CREATE TABLE afvd_p (id INT PRIMARY KEY);
CREATE TABLE afvd (a INT);
ALTER TABLE afvd ADD CONSTRAINT afvd_b_fkey FOREIGN KEY (a) REFERENCES afvd_p(id);
ALTER TABLE afvd ADD COLUMN b INT REFERENCES afvd_p(id);

-- @test
SELECT constraint_name::text AS constraint_name, constraint_type::text AS constraint_type
FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'afvd' AND constraint_type = 'FOREIGN KEY'
ORDER BY constraint_name;
