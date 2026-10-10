-- The **left** half of the same rule, isolated: the key is the `text` `?` registers, so
-- only the left operand can refuse the pair. Without this the right-operand check alone
-- reddens every other fixture of the family and the left one is pinned by none.
-- @test
-- @expect-error: no integer ? text operator
-- @expect-message
SELECT 1 ? 'a'::text;
