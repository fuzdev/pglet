-- The one spelling the prefix rule does **not** reach: `'o'` cannot choose
-- between `on` and `off`, so 18.6 compares two characters there and refuses it.
-- @expect-error: invalid input syntax for type boolean
-- @test
SELECT 'o'::bool;
