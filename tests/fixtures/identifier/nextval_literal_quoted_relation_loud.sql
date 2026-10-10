-- The two name positions must agree, and this is the statement that proves they do
-- from the disagreeing side: `CREATE SEQUENCE MySeq` folds to `myseq` in the
-- **identifier** position, so the **literal** `'"MySeq"'` — quoted, therefore exact —
-- names nothing (`42P01` on both).
-- Its twin `sequences/nextval_literal_unquoted_case_folds.sql` reads the same sequence
-- through an unquoted literal and finds it. Neither fixture alone separates the rules:
-- one passes if the literal folds unconditionally, the other if it never folds.
-- @setup
CREATE SEQUENCE MySeq;

-- @test
SELECT nextval('"MySeq"');
-- @expect-error: a quoted literal part is exact, and the sequence was registered as `myseq` (42P01)
