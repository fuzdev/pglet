-- The typed-constant position takes the same refusal, which is the whole of what
-- `gram.y`'s `ConstCharacter` does *not* narrow: it clears `CharacterWithoutLength`'s
-- `bpchar(1)` default and passes `CharacterWithLength` straight to `typenameTypeMod`. So
-- `char 'ab'` is `ab` on both engines and `char(0) 'ab'` is this `22023` on both.
-- @expect-error: a character length below 1 at a typed constant
-- @expect-message
-- @test
SELECT char(0) 'ab' AS never_reached;
