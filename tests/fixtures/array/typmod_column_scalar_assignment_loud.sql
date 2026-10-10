-- The column's declared type is the **array**, so a bare scalar assigned to it is
-- `42804` — the same answer a modifier-less `numeric[]` column gives. It used to be
-- accepted, storing an unrounded scalar into a column declared `numeric(4,1)[]`.
-- @setup
CREATE TABLE tmnu2 (a numeric(4,1)[]);
-- @expect-error: a numeric is not a numeric[]
-- @test
INSERT INTO tmnu2 VALUES (1.25);
