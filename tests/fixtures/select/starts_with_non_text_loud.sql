-- `starts_with` has exactly one signature, `starts_with(text, text)`: a non-text
-- argument on either side is 42883, never a silent `false`.
-- @test
-- @expect-error: no starts_with(integer, text)
SELECT starts_with(1::int, 'a');
