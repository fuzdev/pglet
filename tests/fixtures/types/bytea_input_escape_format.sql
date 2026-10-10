-- A string with no `\x` at its start is read in `byteain`'s escape format: each byte of the
-- text is itself, `\\` is one backslash and `\ooo` is the byte of that octal value. Nothing
-- is trimmed and nothing is hex-decoded, so a run of hex digits is its own characters.
-- @test
SELECT
  'abcd'::bytea AS even_hex_digits,
  'abc'::bytea AS odd_hex_digits,
  'AB12'::bytea AS upper_hex_digits,
  'hello'::bytea AS plain,
  ''::bytea AS empty,
  ' '::bytea AS one_space,
  '  abcd '::bytea AS spaces_kept,
  'x6162'::bytea AS no_backslash,
  'héllo €'::bytea AS non_ascii,
  '\141\142'::bytea AS octal,
  '\000\377\177'::bytea AS octal_bounds,
  '\1411'::bytea AS octal_then_digit,
  'a\\b'::bytea AS backslash,
  '\\'::bytea AS backslash_alone,
  '\\x6162'::bytea AS escaped_backslash_then_x,
  '\\141'::bytea AS escaped_backslash_then_digits;
