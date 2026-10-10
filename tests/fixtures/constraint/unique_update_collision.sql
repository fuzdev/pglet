-- A valid INSERT then an UPDATE that moves one row's key onto another's violates
-- the unique constraint.
-- @profile-lock
-- @setup
CREATE TABLE uq_upd (id INT PRIMARY KEY, name TEXT);
INSERT INTO uq_upd VALUES (1, 'a'), (2, 'b');

-- @test
-- @expect-error: UPDATE onto an existing key violates unique constraint
UPDATE uq_upd SET id = 2 WHERE id = 1;
