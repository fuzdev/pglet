-- The mirror of `default_between_two_columns_takes_declared_order_int_first.sql`:
-- the same two bad defaults in the other order answer the other SQLSTATE.
-- @expect-error: the first declared column's default is cooked first
-- @test
CREATE TABLE def_order (a DATE DEFAULT 'y', b INT DEFAULT 'x');
