-- 18.6 finishes lexing a string token -- raising a bad escape's error as it goes --
-- before it reads what stands after the closing quote, so this is `22P05` and not the
-- trailing junk's `22P02`. It agreed before the whitespace narrowing only because the
-- form feed was trimmed away; keeping it agreeing is why the reader decodes a string's
-- body before it refuses what follows the quote.
-- @expect-error: the code-point-zero escape, not the junk after it
-- @expect-message
-- @test
SELECT (E'["\\u0000"\x0c]')::jsonb;
