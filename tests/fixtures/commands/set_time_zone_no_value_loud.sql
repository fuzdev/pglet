-- `SET TIME ZONE` with no `zone_value` is a syntax error on both engines. pglet reads the
-- spelling as the `SET timezone = <value>` it means, and a missing value must stay loud there.
-- @test
-- @expect-error: syntax error
SET TIME ZONE;
