-- @test
-- raw_bytea round-trips via the binary path, the ::text casts compare as TEXT
-- (the text-format wire encoder is unit-tested in pglet_server::encoding)
SELECT
  '\xDEADBEEF'::bytea AS raw_bytea,
  '\xDEADBEEF'::bytea::text AS hex_text,
  '\x'::bytea::text AS empty_hex,
  '\x00ff10'::bytea::text AS mixed_hex;
