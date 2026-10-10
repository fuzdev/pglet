-- The typed-constant spelling `bytea '…'` reads its string as the cast does.
-- @test
SELECT
  bytea 'abcd' AS even_hex_digits,
  bytea 'hello' AS plain,
  bytea ' ' AS one_space,
  bytea '\141\\' AS escapes,
  bytea '\x6162 63' AS hex;
