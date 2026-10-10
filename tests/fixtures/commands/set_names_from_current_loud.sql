-- `SET NAMES FROM CURRENT` is `set_rest_more`'s `var_name FROM CURRENT` naming the setting `names`,
-- which 18.6 does not have: its `42704`, not a syntax error.
-- @test
-- @expect-error: unrecognized configuration parameter "names"
SET NAMES FROM CURRENT;
