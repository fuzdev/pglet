-- A value that is not a Boolean is 18.6's own `22023 parameter "standard_conforming_strings"
-- requires a Boolean value`, raised before pglet's refusal of `off` (`0A000`) is asked — that
-- one is reserved for a value 18.6 takes (`set_standard_conforming_strings_off_loud.sql`).
-- @test
-- @expect-error: parameter "standard_conforming_strings" requires a Boolean value
-- @expect-message
SET standard_conforming_strings = 'maybe';
