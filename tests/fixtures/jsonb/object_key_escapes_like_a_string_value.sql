-- 18.6 hands an object key and a string value to the same `jsonb_put_escaped_value`, so
-- a key holding a `"`, a `\`, a control character or a non-ASCII character is written
-- with the spelling its value would take. Written raw instead, the first column is not
-- valid JSON at all and the second is another key's spelling.
-- @test
SELECT '{"a\"b": 1}'::jsonb::text AS quote,
       '{"a\\b": 1}'::jsonb::text AS backslash,
       '{"a\tb": 1}'::jsonb::text AS tab,
       '{"a\u0001b": 1}'::jsonb::text AS low_control,
       '{"a/\u00e9b": 1}'::jsonb::text AS verbatim,
       '{"ab": 1}'::jsonb::text AS plain;
