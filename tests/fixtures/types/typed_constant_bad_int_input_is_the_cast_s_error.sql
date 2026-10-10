-- The `22P02` half of the pair (see
-- `typed_constant_bad_input_is_the_cast_s_error.sql`): an integer typed constant
-- whose literal is not an integer fails with the same code `'x'::int4` fails with.
-- @test
-- @expect-error: invalid input syntax for type integer
SELECT INT4 'x';
