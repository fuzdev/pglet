-- `{xeunicodefail}` is matched in the `xeu` state too, so a malformed escape after a
-- first surrogate half is the escape's `22025` and not the pairing's `42601` -- the two
-- verdicts meet here and the malformed-escape one wins.
-- @test
-- @expect-error: the malformed escape outranks the pending pair
-- @expect-message
SELECT E'\uD83D\u12';
