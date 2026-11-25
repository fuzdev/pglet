-- A backslash before anything but a backslash or three octal digits is `22P02`, and `\x`
-- is not the hex prefix in this format.
-- @expect-error: invalid input syntax for type bytea
-- @test
SELECT decode('\x41', 'escape');
