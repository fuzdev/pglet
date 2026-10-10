-- The typed-constant position takes the same refusal. It is the position that has to
-- *commit*: pglet reads a multi-word type name speculatively and puts the cursor back
-- when no string follows, and a restore would swallow this verdict.
-- @expect-error: a float precision below 1 bit at a typed constant
-- @expect-message
-- @test
SELECT FLOAT(0) '0.5' AS never_reached;
