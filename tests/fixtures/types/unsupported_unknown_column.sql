-- An unrecognized type fails loud rather than degrading to TEXT with a warning.
-- @setup

-- @test
-- @expect-error: type "foobar" is not supported
CREATE TABLE unsupported_unknown (x foobar);
