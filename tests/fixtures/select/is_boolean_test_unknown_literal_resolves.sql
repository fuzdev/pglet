-- The operand position resolves Postgres's `unknown`: a bare string literal is
-- *converted* to boolean rather than typed `text` and refused, so `'t' IS TRUE` is
-- `t` on both engines. The refusal half is
-- `is_boolean_test_unknown_literal_bad_value_is_22p02.sql`; the typed-operand half is
-- `is_boolean_test_non_boolean_operand_is_42804.sql`.
-- @test
SELECT 't' IS TRUE AS a, 'f' IS NOT TRUE AS b, 'yes' IS TRUE AS c;
