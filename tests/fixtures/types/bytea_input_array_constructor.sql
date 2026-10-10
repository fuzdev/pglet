-- A string inside an `ARRAY[…]` constructor of `bytea` elements, cast or resolved beside a
-- typed element, is read as `bytea`.
-- @test
SELECT
  ARRAY['abcd', '\x6162', ' ', '\141']::bytea[] AS cast_constructor,
  ARRAY['abcd'::bytea, 'cafe'] AS typed_sibling,
  ARRAY['abcd'::bytea] = ARRAY['\x61626364'::bytea] AS equal_bytes,
  ARRAY['abcd'::bytea] = ARRAY['\xabcd'::bytea] AS hex_reading;
