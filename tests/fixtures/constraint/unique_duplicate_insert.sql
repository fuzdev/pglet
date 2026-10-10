-- A duplicate single-column UNIQUE value violates the unique constraint.
-- @profile-lock
-- @setup
CREATE TABLE uq_dup (id INT, email TEXT UNIQUE);
INSERT INTO uq_dup VALUES (1, 'x@y');

-- @test
-- @expect-error: duplicate UNIQUE value violates unique constraint
INSERT INTO uq_dup VALUES (2, 'x@y');
