-- pglet-stricter, and loud: an integer spelling of on (`+1`, which 18.6's grammar reads as the
-- integer `1`) is refused `0A000` like `on` itself. Documentary: 18.6 accepts, so any pglet
-- error passes.
-- @test
-- @expect-error: quote_all_identifiers = on is refused here
SET quote_all_identifiers = +1;
