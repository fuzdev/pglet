-- The table argument is read as a qualified name: an unquoted part folded, whitespace
-- around it skipped, `public.` and the current database's `<db>.public.` accepted, and
-- a keyword needs no quoting there.
-- @setup
CREATE TABLE ss (id serial);
CREATE TABLE "select" (id serial);
-- @test
SELECT pg_get_serial_sequence('SS', 'id') AS folded, pg_get_serial_sequence('public.ss', 'id') AS qualified,
  pg_get_serial_sequence('"ss"', 'id') AS quoted, pg_get_serial_sequence(' ss ', 'id') AS spaced,
  pg_get_serial_sequence(current_database() || '.public.ss', 'id') AS three_part,
  pg_get_serial_sequence('select', 'id') AS keyword_unquoted,
  pg_get_serial_sequence(quote_ident('public') || '.' || quote_ident('ss'), 'id') AS built;
