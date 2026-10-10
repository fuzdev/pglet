-- A precision 18.6's message prints **signed**, which a character length cannot reach:
-- `opt_type_modifiers` is an `expr_list` and the scale may be negative, so the `-` is
-- part of the modifier grammar here where `CharacterWithLength`'s `Iconst` refuses it.
-- @expect-error: a negative NUMERIC precision
-- @expect-message
-- @test
SELECT 1.5::numeric(-1) AS never_reached;
