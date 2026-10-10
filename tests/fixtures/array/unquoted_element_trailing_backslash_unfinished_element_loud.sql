-- The element the backslash cuts short is never converted: the body's end is refused
-- ahead of the input error that element's text would raise.
-- @test
-- @expect-error: the unfinished element's odd hex digit count is not reached; 22P02
SELECT '{\\x616\}'::bytea[];
