-- A list assigned to a setting that takes one value is refused before the value is read —
-- 18.6's `flatten_set_variable_args`, `22023 SET lock_timeout takes only one argument`.
-- pglet answered `42601` here, reading the list as a malformed value.
-- @test
-- @expect-error: SET lock_timeout takes only one argument
SET lock_timeout = 1, 2;
