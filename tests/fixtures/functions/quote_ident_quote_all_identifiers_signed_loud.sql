-- A signed number reaches the Boolean reader as 18.6's grammar builds it (`NumericOnly`):
-- `-1` is not a Boolean, so it is 18.6's `22023`. The reader used to see two tokens and pass the
-- statement as `SET`.
-- @test
-- @expect-error: parameter "quote_all_identifiers" requires a Boolean value
SET quote_all_identifiers = -1;
