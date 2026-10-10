-- `quote_all_identifiers` off — its default, spelled as a prefix and as `0` — is what pglet
-- does, so it is accepted and `quote_ident` answers as before.
-- @setup
-- (a first statement that is not a `SET`: the header above reaches the setup's first
-- statement, and a comment-prefixed `SET` is row 23's classifier gap)
SELECT 1;
SET quote_all_identifiers = 'of';
SET quote_all_identifiers = 0;
-- @test
SELECT quote_ident('abc') AS plain, quote_ident('select') AS keyword;
