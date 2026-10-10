-- Updating a row's email so its LOWER key collides with another row's key
-- violates the expression unique index: row 2's email is changed to 'A@X', whose
-- lowercased key matches row 1's existing 'a@x'.
-- @profile-lock
-- @setup
CREATE TABLE expr_lower_update (id INT, email TEXT);
CREATE UNIQUE INDEX ux_expr_lower_update ON expr_lower_update (LOWER(email));
INSERT INTO expr_lower_update VALUES (1, 'a@x'), (2, 'b@x');

-- @test
-- @expect-error: UPDATE moving LOWER(email) onto an existing key collides
UPDATE expr_lower_update SET email = 'A@X' WHERE id = 2;
