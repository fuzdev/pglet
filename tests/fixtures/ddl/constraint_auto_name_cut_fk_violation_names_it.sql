-- A `23503` names the constraint the catalog holds — the cut name — in its message and in
-- its constraint-name field.
-- @setup
CREATE TABLE cac14p (id INT PRIMARY KEY);
CREATE TABLE cac14_tttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT REFERENCES cac14p);
-- @expect-error: 23503 naming the 63-byte constraint
-- @expect-message
-- @profile-lock
-- @test
INSERT INTO cac14_tttttttttttttttttttttttttttttttttttttttttttttttttttttt VALUES (7);
