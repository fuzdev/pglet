-- Without `IF EXISTS`, a name no constraint of the table holds is `42704`, even when
-- another table has a constraint of that name.
-- @expect-error: the constraint does not exist on this table
-- @expect-message
-- @setup
CREATE TABLE dcml_a (x INT CHECK (x > 0));
CREATE TABLE dcml_b (x INT CONSTRAINT dcml_elsewhere CHECK (x > 0));

-- @test
ALTER TABLE dcml_a DROP CONSTRAINT dcml_elsewhere;
