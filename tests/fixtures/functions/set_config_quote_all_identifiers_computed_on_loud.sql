-- pglet-stricter, and loud: the `set_config` route of `quote_all_identifiers`, a value computed
-- at run time, is refused as `SET quote_all_identifiers = on` is
-- (`quote_ident_quote_all_identifiers_on_loud.sql`). On 18.6 it answers `on` and every name
-- 18.6 renders afterwards comes back quoted; pglet honors the setting in none of its readers.
-- Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: quote_all_identifiers = on is refused here
SELECT set_config('quote_all_identifiers', lower('ON'), false);
