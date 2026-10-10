-- The `SET LOCAL` spelling of `set_guc_quoted_mixed_case_quote_all_identifiers_loud.sql`, in the
-- harness's block: a quoted upper-case name is the same setting, and turning it on is refused.
-- Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: quote_all_identifiers = on is refused here, whatever the name's spelling
SET LOCAL "QUOTE_ALL_IDENTIFIERS" TO true;
