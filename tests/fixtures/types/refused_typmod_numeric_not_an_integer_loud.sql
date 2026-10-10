-- The `int4` coercion every component goes through first (`ArrayGetIntegerTypmods`) is
-- part of the modifier's validation, and it has its **own** SQLSTATE: a component that is
-- not an integer is `22P02`, not the `22023` either window raises. `numeric`'s grammar is
-- an `expr_list` rather than an `Iconst`, so this is the one family a non-integer
-- component reaches at all.
-- @expect-error: a NUMERIC component that is not an integer
-- @expect-message
-- @test
SELECT 1.5::numeric(1.5) AS never_reached;
