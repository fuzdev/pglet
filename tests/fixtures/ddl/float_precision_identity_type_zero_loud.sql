-- An identity column's `(AS <type>)` is the second position that resolves no type name,
-- and it agrees for the same reason as
-- `ddl/float_precision_sequence_type_zero_loud.sql` — the refusal is the grammar's. Its
-- in-range spelling parts under a *different* code again (`42601 conflicting or
-- redundant options`, from a duplicate-option scan), which is what makes the split the
-- position's rather than the type's.
-- @expect-error: a float precision below 1 bit at an identity column's type
-- @expect-message
-- @test
CREATE TABLE zi (id int GENERATED ALWAYS AS IDENTITY (AS FLOAT(0)), b int);
