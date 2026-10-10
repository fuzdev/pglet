-- A typed constant's conversion failure is the *cast's* failure, with the cast's
-- SQLSTATE: `22007` for a malformed date. Pairs with
-- `typed_constant_bad_int_input_is_the_cast_s_error.sql`, which pins the other code
-- the same grammar reaches — one fixture per SQLSTATE, since `@expect-error`
-- compares only that.
-- @test
-- @expect-error: invalid input syntax for type date
SELECT DATE 'not-a-date';
