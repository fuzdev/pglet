-- A `23505` names the constraint the catalog holds — the cut name — in its message and in
-- its constraint-name field.
-- @setup
CREATE TABLE cac12_tttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT UNIQUE);
INSERT INTO cac12_tttttttttttttttttttttttttttttttttttttttttttttttttttttt VALUES (1);
-- @expect-error: 23505 naming the 63-byte constraint
-- @expect-message
-- @profile-lock
-- @test
INSERT INTO cac12_tttttttttttttttttttttttttttttttttttttttttttttttttttttt VALUES (1);
