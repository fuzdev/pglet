-- The mirror of `check_date_part_binding_in_field`: the operand-type error
-- (`integer -> unknown`) now sits in the *field* slot, which Postgres resolves
-- first, so it wins over the unresolvable column in the source slot.
-- @test
-- @expect-error: no integer -> text operator
CREATE TABLE ckdp2 (a INT, CHECK (date_part((a -> 'k'), bogus) > 0));
