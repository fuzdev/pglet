-- `opt_float` spells its window with one `Iconst`, so a fractional precision is a
-- **syntax** error on both engines rather than a rounded or truncated one.
-- @no-error-fields: both engines send a `P`, and pglet's reader reports one token later —
--   it validates the number after consuming it, where 18.6's lexer refuses the `ICONST`
--   at the number itself. The same offset the temporal spellings' fixtures record.
-- @expect-error: a float precision that is not an integer
-- @test
SELECT 1::float(1.5) AS never_reached;
