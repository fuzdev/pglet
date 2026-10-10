-- pglet-stricter, and loud: turning `quote_all_identifiers` on is refused `0A000`. On 18.6 it
-- makes `quote_ident('abc')` answer `"abc"`, and every other name 18.6 renders comes back quoted
-- too; pglet honors it in none of its readers, so accepting it would leave each answering the
-- unquoted spelling with no error. Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: quote_all_identifiers = on is refused here
SET quote_all_identifiers = on;
