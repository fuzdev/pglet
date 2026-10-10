-- In an unquoted array element a backslash takes the next character as it is: an escaped
-- backslash is one backslash, an escaped comma does not end the element, an escaped space
-- is not trailing whitespace, and an escaped `NULL` is the four characters.
-- @test
SELECT
  '{a\\b,"a\\b"}'::text[] AS escaped_backslash,
  '{a\,b,c}'::text[] AS escaped_comma,
  '{a\ ,\  z,  x y  }'::text[] AS escaped_space,
  '{\NULL,NULL,nu\ll}'::text[] AS escaped_null,
  '{a\"b,\x41}'::text[] AS escaped_other,
  '{1\2,3}'::int[] AS escaped_digit;
