-- The same scan with the order swapped: now the NUL *is* first, so it is the failure
-- and the high byte behind it is never reached. This pair is what separates a
-- left-to-right scan from an eager per-escape check.
-- @test
-- @expect-error: the NUL is first here
-- @expect-message
SELECT E'\x00\xE9';
