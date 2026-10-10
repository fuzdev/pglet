-- A UNIQUE NUMERIC column: two values that are equal by value but differ in
-- display scale (1.5 and 1.50) collide, so the second insert violates the
-- constraint — exercising the order-preserving index key's equality collapse.
-- @profile-lock
-- @setup
CREATE TABLE u (amount NUMERIC UNIQUE);
INSERT INTO u VALUES (1.5);
-- @expect-error: 1.50 equals the existing 1.5 → unique_violation (23505)
-- @test
INSERT INTO u VALUES (1.50);
