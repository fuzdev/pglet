-- The operand is an operator result rather than a column, which the narrow
-- plan-time readers beside this gate answer `None` for — the complete type walk
-- is what makes it `42804` here rather than a zero-row silence.
-- @setup
CREATE TABLE bcn2 (a INT);
INSERT INTO bcn2 VALUES (9);
-- @expect-error: argument of WHERE must be type boolean
-- @test
SELECT count(*) FROM bcn2 WHERE a + 1;
