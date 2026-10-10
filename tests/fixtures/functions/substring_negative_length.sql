-- A negative substring length is an error (22011, substring_error) in both pglet
-- and Postgres — not a silently clamped empty string.
-- @test
-- @expect-error: negative substring length not allowed
SELECT substring('12345' FROM 2 FOR -1);
