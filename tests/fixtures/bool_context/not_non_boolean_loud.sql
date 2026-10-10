-- `NOT` over a non-boolean is 18.6's `42804` — an ill-typed operand — and not a
-- `0A000`, which would say the engine lacks a feature.
-- @expect-error: argument of NOT must be type boolean
-- @test
SELECT NOT 1;
