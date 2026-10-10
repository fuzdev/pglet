-- @test
-- @text
-- A jsonb string containing control chars must render as valid JSON on the wire
-- (Postgres `escape_json`: `\t`/`\n`/`\r`/`\b`/`\f` and `\u00XX` for other C0),
-- not raw bytes. Compared over the simple-query *text* protocol (`@text`), which
-- exercises `value_to_text` → `to_json_string`'s escaping — the path the binary
-- default doesn't, and the one that produced invalid JSON before the fix.
SELECT '{"s":"a\tb\ncd"}'::jsonb AS j;
