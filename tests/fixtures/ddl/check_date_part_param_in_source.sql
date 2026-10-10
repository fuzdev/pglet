-- The mirror of `check_date_part_param_in_field`: the operand-type error is in the
-- field slot, which resolves first, so `$1` in the source slot is never reached.
-- @test
-- @expect-error: no integer -> text operator
CREATE TABLE ckdp4 (a INT, CHECK (date_part((a -> 'k'), $1) > 0));
