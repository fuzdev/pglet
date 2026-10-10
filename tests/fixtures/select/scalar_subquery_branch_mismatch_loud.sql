-- A `text` scalar subquery beside an `integer` constant has no common type.
-- @expect-error: COALESCE types text and integer cannot be matched
-- @test
SELECT coalesce((SELECT '7'), 1);
