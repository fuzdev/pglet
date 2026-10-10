-- A `23514` names the constraint the catalog holds — the cut name — in its message and in
-- its constraint-name field.
-- @setup
CREATE TABLE cac13_tttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT CHECK (cooooooooooooooooooooooooooooooooooooooo > 0));
-- @expect-error: 23514 naming the 63-byte constraint
-- @expect-message
-- @test
INSERT INTO cac13_tttttttttttttttttttttttttttttttttttttttttttttttttttttt VALUES (-1);
