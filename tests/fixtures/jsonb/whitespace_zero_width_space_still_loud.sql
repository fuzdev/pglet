-- The boundary the narrowing must not overshoot in the other direction: `U+200B` carries
-- no `White_Space` property, so it was refused by both engines before the narrowing and
-- still is. It is also the stand-in that measured the hazard -- with the trim narrowed,
-- every non-JSON whitespace character reaches the reader exactly as this one always did.
-- @expect-error: a zero-width space where a token starts
-- @expect-message
-- @test
SELECT (E'\u200b1')::jsonb;
