-- The control's refusal half: an unresolved literal that is not a boolean
-- spelling is the conversion's `22P02`, not the gate's `42804`.
-- @setup
CREATE TABLE bcag6 (b TEXT);
INSERT INTO bcag6 VALUES ('x'), ('y');
-- @expect-error: invalid input syntax for type boolean
-- @test
SELECT b FROM bcag6 GROUP BY b HAVING 'x' IS TRUE;
