-- An `unknown` operand that is not a boolean spelling fails in the **conversion**,
-- not the operand gate: `22P02`, not `42804`. The two codes are what separate an
-- unknown literal from a typed one at this position.
-- @test
-- @expect-error: invalid input syntax for type boolean
SELECT 'x' IS TRUE;
