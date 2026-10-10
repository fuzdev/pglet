-- A list is not a Boolean: `SET quote_all_identifiers = on, off` is 18.6's `22023 SET
-- quote_all_identifiers takes only one argument`, answered before the value is read. The
-- reader used to see no single value and pass the statement as `SET`.
-- @test
-- @expect-error: SET quote_all_identifiers takes only one argument
SET quote_all_identifiers = on, off;
