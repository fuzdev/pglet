-- The `ALTER` routes are unaffected by the `CREATE TABLE` blindness and this pins
-- that rather than assuming it: the table is already in the catalog here, so the one
-- `relation_name_taken` check inside the declared branch sees it, and the collision is
-- the *sequence's* (42P07, before the column is touched) rather than the heap's.
-- @setup
CREATE TABLE agown (x INT NOT NULL);
-- @test
ALTER TABLE agown ALTER COLUMN x ADD GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME agown);
-- @expect-error: relation already exists (42P07)
