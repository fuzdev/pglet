-- A dropped constraint's name is free again: the next unnamed foreign key on the same
-- column takes `<table>_<column>_fkey` itself rather than a digit-suffixed one, and a
-- declared name can be reused.
-- @setup
CREATE TABLE dcfn_p (id INT PRIMARY KEY);
CREATE TABLE dcfn_c (a INT REFERENCES dcfn_p(id), b INT CONSTRAINT dcfn_named REFERENCES dcfn_p(id));

-- @test
ALTER TABLE dcfn_c DROP CONSTRAINT dcfn_c_a_fkey, DROP CONSTRAINT dcfn_named;
ALTER TABLE dcfn_c ADD FOREIGN KEY (a) REFERENCES dcfn_p(id);
ALTER TABLE dcfn_c ADD CONSTRAINT dcfn_named FOREIGN KEY (b) REFERENCES dcfn_p(id) ON DELETE CASCADE;
SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint
WHERE conname LIKE 'dcfn%' ORDER BY conname;
