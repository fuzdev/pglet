-- An array literal's elements are read by the same reader.
-- @test
-- @expect-error: "1e39" is out of range for type real (22003)
-- @expect-message
SELECT '{1,1e39}'::real[];
