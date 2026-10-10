-- A quoted mixed-case spelling of `client_encoding` reaches the UTF-8-only refusal
-- (`set_client_encoding_non_utf8.sql`): 18.6 matches the name case-insensitively whatever its
-- quoting. Read verbatim it answered `SET` and the connection went on sending UTF-8 bytes to a
-- client told to decode LATIN1. Documentary: 18.6 accepts, so any pglet error passes.
-- @test
-- @expect-error: pglet is UTF8-only, whatever the name's spelling
SET "Client_Encoding" = 'LATIN1';
