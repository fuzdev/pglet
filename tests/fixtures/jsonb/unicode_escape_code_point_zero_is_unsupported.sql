-- A jsonb string's \u0000 escape is 18.6's `22P05 unsupported Unicode escape sequence`
-- (`JSON_UNICODE_CODE_POINT_ZERO`): the decoded string would become a `text` value, and a
-- NUL is not valid in the server encoding. Not the `22P02` the other malformed escapes
-- take, and not the `42601` a string literal's \u0000 takes.
-- @test
-- @expect-error: code point zero is untranslatable
-- @expect-message
SELECT '"a\u0000b"'::jsonb;
