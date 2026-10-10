-- A string longer than the word it prefixes is not a prefix: the comparison
-- reaches the word's own terminator.
-- @expect-error: invalid input syntax for type boolean
-- @test
SELECT 'truex'::bool;
