-- A nested set operation's column was resolved when it was built: the all-unknown
-- `'1' UNION ALL '2'` is `text`, so it has no common type with `3`.
-- @expect-error: UNION types text and integer cannot be matched
-- @test
SELECT '1' UNION ALL SELECT '2' UNION ALL SELECT 3;
