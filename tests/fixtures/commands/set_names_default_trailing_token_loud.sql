-- `SET NAMES DEFAULT` followed by another token is 18.6's `42601`: `DEFAULT` is the whole value.
-- @test
-- @expect-error: syntax error at or near "garbage"
SET NAMES DEFAULT garbage;
