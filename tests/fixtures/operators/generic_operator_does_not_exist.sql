-- An operator name pglet implements nothing for is refused where 18.6 refuses it: in
-- the statement's analysis, `42883`, with 18.6's own sentence over the operand types.
-- Every `op_chars` spelling lexes on both engines (`scan.l`'s `{operator}` rule), so
-- this is no scanner error.
-- @test
-- @expect-error: operator does not exist: integer ! integer
-- @expect-message
SELECT 1 ! 1;
