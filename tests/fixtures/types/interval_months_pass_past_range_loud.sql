-- The months pass lands on 294277-01-01, the first day past 18.6's timestamp range, and
-- is refused there; `interval '3411335 months -103830377 days'` is a value.
-- @expect-error: timestamp out of range
-- @expect-message
-- @test
SELECT timestamp '9999-01-01' + interval '3411336 months -103830408 days';
