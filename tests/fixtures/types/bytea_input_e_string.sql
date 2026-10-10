-- The lexer has taken one level of backslashes out of an `E''` string before the input
-- function reads it, and none out of a plain one: `E'\\001'` reaches it as `\001`, one
-- byte, where `'\\001'` reaches it as written, an escaped backslash and three digits.
-- @test
SELECT
  E'\\001'::bytea AS e_octal,
  '\\001'::bytea AS plain_escaped_backslash,
  E'\\\\001'::bytea AS e_escaped_backslash,
  '\001'::bytea AS plain_octal,
  E'\001'::bytea AS e_lexer_octal,
  E'\\x6162'::bytea AS e_hex,
  E'\x61b'::bytea AS e_lexer_hex,
  E'a\nb'::bytea AS e_newline,
  E'\\\\'::bytea AS e_backslash;
