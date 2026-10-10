-- A prefix of `true` is on to 18.6's Boolean reader (`'tr'`), so it is refused the same way.
-- Documentary: 18.6 accepts.
-- @test
-- @expect-error: quote_all_identifiers = on is refused here
SET quote_all_identifiers = 'tr';
