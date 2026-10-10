-- A nested literal's elements are converted as they are read, so an element's own input
-- error comes ahead of the refusal of the second dimension.
-- @test
-- @expect-error: the element's own 22007
SELECT '{{x}}'::date[];
