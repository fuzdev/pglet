-- The math arm (`math_call`). Its argument is a bare `'x'`, which is `unknown` on 18.6
-- and so lets a signature resolve — the quantifier is what fails, at 42809, rather than
-- the operand type. That the signature really does resolve is measured by the `ALL`
-- spelling: `abs(ALL 'x')` is 22P02 there, an *input* conversion failing at execution.
-- pglet used to answer this statement with its **eval-time** type
-- gate (`0A000 abs() requires a numeric argument`), which is a later phase, so this
-- fixture is also the pin that the verdict outranks the type gates.
-- @test
-- @expect-error: DISTINCT specified, but abs is not an aggregate function
SELECT abs(DISTINCT 'x');
