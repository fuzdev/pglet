-- The partial `LOWER(email) WHERE email IS NOT NULL` index still enforces among
-- the non-NULL rows: 'A@x' and 'a@x' both satisfy the predicate and fold to the
-- same lowercased key, so the second insert collides.
-- @profile-lock
-- @setup
CREATE TABLE expr_lower_partial_collide (id INT, email TEXT);
CREATE UNIQUE INDEX ux_expr_lower_partial_collide
  ON expr_lower_partial_collide (LOWER(email)) WHERE email IS NOT NULL;
INSERT INTO expr_lower_partial_collide VALUES (1, NULL), (2, NULL), (3, 'A@x');

-- @test
-- @expect-error: LOWER(email) collision among non-NULL rows violates the index
INSERT INTO expr_lower_partial_collide VALUES (4, 'a@x');
