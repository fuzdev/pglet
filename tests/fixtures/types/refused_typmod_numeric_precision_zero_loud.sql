-- The rule is the `(…)`'s and not `char`'s: `numeric`'s precision window is
-- `numerictypmodin`'s `1 .. 1000` (`numeric.c`'s `make_numeric_typmod_safe`, this very
-- refusal's `LOCATION` on the oracle), the same silence lived at the same resolver, and `1.5::numeric(0)` answered
-- `1.5` here where 18.6 raises.
-- @expect-error: a NUMERIC precision below 1
-- @expect-message
-- @test
SELECT 1.5::numeric(0) AS never_reached;
