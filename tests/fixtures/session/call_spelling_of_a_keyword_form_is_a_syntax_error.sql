-- The five RESERVED_KEYWORD forms have no call spelling at all: 18.6's scanner
-- makes them keywords, so the `(` is a syntax error and never a wrong-arity
-- resolution. `current_schema` is the exception and has its own fixture.
-- @expect-error: current_user() is a syntax error, not a function call
-- @test
SELECT current_user();
