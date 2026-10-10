-- The mirror: the over-long list on the **second** declaration, so declaration order and
-- Postgres order agree. The control that identifies the ordering rather than the gate.
-- @expect-error: the duplicate name outranks the later declaration's arity error
-- @expect-message
-- @test
WITH c AS (SELECT 1), c(a, b) AS (SELECT 2) SELECT 1 AS only_column;
