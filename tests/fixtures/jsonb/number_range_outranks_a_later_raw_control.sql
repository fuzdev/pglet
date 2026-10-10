-- The other order of the same pair: the number is reached first, so its `22003` wins. The
-- sentence is 18.6's own on both sides — the weight and display-scale caps answer
-- `Error::NumericFormatOverflow`, not a `numeric out of range` of pglet's own — so this
-- locks it too.
-- @expect-error: the numeric overflow in the first element
-- @expect-message
-- @test
SELECT (E'[1e200000, "a\tb"]')::jsonb;
