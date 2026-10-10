-- The same bound over a name that takes no `*` at all. It was loud here before, at the
-- wrong code (0A000, the generic wildcard refusal), which is why it needs its own
-- fixture beside the `count` one: the two reach different arms of the wildcard block and
-- only one of them was silent. 42601 on both.
-- @test
-- @expect-error: syntax error at or near "*"
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 at the `*`, pglet at the closing paren
SELECT upper(DISTINCT *);
