-- `SET NAMES` takes one value (`gram.y`'s `opt_encoding`: a string, `DEFAULT`, or nothing), so a
-- second token is 18.6's `42601`, as for the `=` form of the same setting.
-- @test
-- @expect-error: syntax error at or near ","
SET NAMES 'utf8', 'latin1';
