-- The refusal is a **parse-analysis** verdict, so it outranks the operand's own runtime
-- error: `1/0` never divides, and answering `22012` here would mean the modifier had been
-- read and dropped before the value was evaluated.
-- @expect-error: the length refusal outranks the division by zero
-- @expect-message
-- @test
SELECT (1/0)::char(0) AS never_reached;
