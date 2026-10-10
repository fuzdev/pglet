-- A value 18.6's Boolean reader refuses — an ambiguous prefix — is its own `22023` on both.
-- @test
-- @expect-error: parameter "quote_all_identifiers" requires a Boolean value
-- @expect-message
SET quote_all_identifiers = 'o';
