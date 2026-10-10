-- Columns are unified and converted one at a time: column 1's `'y'` is refused before
-- column 2's `'x'` is reached.
-- @expect-error: invalid input syntax for type integer
-- @expect-message
-- @test
SELECT 1, 'x' UNION ALL SELECT 'y', 2;
