-- The `$N`-placeholder half of the `check_date_part_binding_in_*` pair: a stored
-- expression binds no parameters, so `$1` is `42P02` at DDL time. Positionally first
-- here, so it wins over the operand-type error in the source slot.
-- @test
-- @expect-error: there is no parameter $1
CREATE TABLE ckdp3 (a INT, CHECK (date_part($1, (a -> 'k')) > 0));
