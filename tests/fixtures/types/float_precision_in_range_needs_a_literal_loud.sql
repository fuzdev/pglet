-- The same commit with an **in-range** precision: `float(1)` is a complete type name
-- and the statement then wants the string literal that would make it a constant, so
-- both engines answer `42601` rather than reading a function call. This is the control
-- that says the commit is keyed on the `(` and not on the refusal.
-- @expect-error: a type name where an expression was needed
-- @test
SELECT float(1);
