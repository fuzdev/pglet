-- The other accept side: the gate reads the *raw* input, not the decoded output. A JSON
-- `\t` escape legitimately decodes to a tab, at a value and at a key alike, and both
-- engines take it. A gate applied to the decoded string would refuse these.
-- @test
SELECT '{"k": "a\tb"}'::jsonb::text AS escaped_value,
       '{"a\tb": 1}'::jsonb::text AS escaped_key,
       '{"k": "a\u0001b"}'::jsonb::text AS escaped_low,
       '{"k": "a\nb\rc\bd\fe"}'::jsonb::text AS every_short_escape;
