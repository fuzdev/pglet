-- A word that is not one of the five targets is a syntax error at *that* token on
-- both engines. It was `42601` here before the statement existed too — at the
-- `DISCARD` — so this fixture pins the SQLSTATE agreement rather than a repair; the
-- message, which 18.6 raises from the scanner with the word as written, is pinned in
-- `catalog::commands`'s unit tests.
-- @test
-- @expect-error: syntax error at or near "FOO"
DISCARD FOO;
