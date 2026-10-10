-- The returned name is 18.6's `quote_identifier` of the sequence: bare, quoted for a keyword
-- or a character outside `a`–`z` / `0`–`9` / `_`, and bare for `action`, a word 18.6 leaves
-- unreserved and pglet's own lexer quotes.
-- @setup
CREATE TABLE "select" (id serial);
CREATE TABLE "my table" ("my col" serial);
CREATE TABLE "Mixed" (id serial);
CREATE TABLE owner_t (c int);
CREATE SEQUENCE action OWNED BY owner_t.c;
-- @test
SELECT pg_get_serial_sequence('"select"', 'id') AS keyword_table, pg_get_serial_sequence('"my table"', 'my col') AS spaces,
  pg_get_serial_sequence('"Mixed"', 'id') AS mixed, pg_get_serial_sequence('owner_t', 'c') AS unreserved;
