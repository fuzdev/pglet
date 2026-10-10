-- `DEALLOCATE` with nothing after it is a syntax error, reported at the terminator.
-- @test
-- @expect-error: syntax error at or near ";"
DEALLOCATE;
