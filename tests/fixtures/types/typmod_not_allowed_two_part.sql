-- The refusal is on the modifier's **presence**, not its shape: a two-part `(p,s)`
-- on a type that carries none is the same `42601`, and so is a negative one.
-- @expect-error: int4 takes no type modifier, whatever its arity
-- @expect-message
-- @test
SELECT 1::int4(5,2) AS never_reached;
