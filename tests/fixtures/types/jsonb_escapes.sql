-- A valid JSONB value with escape sequences, read back via `->>` extraction
-- (the raw decoded text). Extraction is stable across jsonb text-format
-- normalization, so it compares byte-for-byte against Postgres and exercises
-- the string unescape decoder (`\t`, `\\`, `\"`, `\/`, `\n`, `\uXXXX`, and
-- UTF-16 surrogate pairs) rather than `to_json_string`'s re-encoding.
-- @test
SELECT
  ('{"s": "a\tb\\c\"d\/e"}'::jsonb) ->> 's' AS escapes,
  ('{"s": "x\ny"}'::jsonb) ->> 's'           AS newline,
  ('{"s": "\u00e9\u00fc"}'::jsonb) ->> 's'  AS unicode,
  ('{"s": "\uD83D\uDE00"}'::jsonb) ->> 's'  AS surrogate,
  -- escaped backslash-t is a literal `\t`, NOT a tab (the old decoder's bug)
  ('{"s": "\\t"}'::jsonb) ->> 's'           AS backslash_t;
