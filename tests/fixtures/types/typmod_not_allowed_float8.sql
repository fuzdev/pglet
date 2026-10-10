-- `float8` carries no modifier. (Bare `FLOAT(n)` is a different grammar production
-- that 18.6 accepts — the precision picks `float4` vs `float8` — and is not this
-- refusal's shape.)
-- @expect-error: float8 takes no type modifier
-- @expect-message
-- @test
SELECT 1.5::float8(3) AS never_reached;
