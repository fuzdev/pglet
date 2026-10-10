-- A long `client_encoding` whose 63rd byte falls inside a character: 18.6 cuts the name before
-- that character, never inside it (`truncate_identifier` clips at a character boundary), and
-- answers its own `22023` for the name it kept, `ab` and thirty `é`.
-- @test
-- @expect-error: invalid value for parameter "client_encoding"
SET client_encoding = 'abéééééééééééééééééééééééééééééééutf8';
