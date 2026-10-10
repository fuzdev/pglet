-- The number alone decides the range verdict, ahead of the trailing-text check, and the
-- message names the number without its padding or the text after it.
-- @test
-- @expect-error: "1e39" is out of range for type real (22003)
-- @expect-message
SELECT ' 1e39x'::real;
