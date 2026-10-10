-- An EXPRESSION unique index over `LOWER(email)` is enforced on the computed key:
-- 'A@x' and 'a@x' fold to the same lowercased key, so the second insert collides.
-- @profile-lock
-- @setup
CREATE TABLE expr_lower_collide (id INT, email TEXT);
CREATE UNIQUE INDEX ux_expr_lower_collide ON expr_lower_collide (LOWER(email));
INSERT INTO expr_lower_collide VALUES (1, 'A@x');

-- @test
-- @expect-error: LOWER(email) collision violates the expression unique index
INSERT INTO expr_lower_collide VALUES (2, 'a@x');
