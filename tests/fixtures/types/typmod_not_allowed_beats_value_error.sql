-- The refusal is a **parse-analysis** verdict, so it outranks the value's own input
-- error: `'x'` is not an integer, and 18.6 never gets that far. Answering `22P02`
-- here would mean the modifier had been read and dropped before the value was.
-- @expect-error: the modifier refusal outranks the bad integer literal
-- @expect-message
-- @test
SELECT 'x'::int4(3) AS never_reached;
