-- The `set_config` spelling of `set_standard_conforming_strings_not_boolean_loud.sql`: 18.6's
-- own `22023`, through the same decider.
-- @test
-- @expect-error: parameter "standard_conforming_strings" requires a Boolean value
-- @expect-message
SELECT set_config('standard_conforming_strings', 'maybe', false);
