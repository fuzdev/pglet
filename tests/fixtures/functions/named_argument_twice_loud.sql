-- An argument name written twice is `42601`, before the function is resolved.
-- @expect-error: argument name used more than once
-- @test
SELECT make_interval(hours => 1, hours => 2);
