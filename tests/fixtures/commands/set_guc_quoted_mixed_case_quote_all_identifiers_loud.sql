-- pglet-stricter, and loud: a quoted mixed-case spelling of `quote_all_identifiers` reaches the
-- same refusal the bare one does (`functions/quote_ident_quote_all_identifiers_on_loud.sql`),
-- because 18.6 matches a setting's name case-insensitively whatever its quoting. Read verbatim,
-- the name walked past the check and answered `SET`, and every name pglet rendered afterwards
-- stayed unquoted where 18.6 quotes it. Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: quote_all_identifiers = on is refused here, whatever the name's spelling
SET "Quote_All_Identifiers" = on;
