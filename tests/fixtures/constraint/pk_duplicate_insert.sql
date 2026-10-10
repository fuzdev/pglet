-- A duplicate primary key value violates the unique constraint.
-- @profile-lock
-- @setup
CREATE TABLE pk_dup (id INT PRIMARY KEY, name TEXT);
INSERT INTO pk_dup VALUES (1, 'a');

-- @test
-- @expect-error: duplicate primary key violates unique constraint
INSERT INTO pk_dup VALUES (1, 'b');
