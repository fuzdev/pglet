-- The plan-time half: over an empty table no row reaches the eval arm, so only the
-- coercion pass can refuse the pair. 18.6 resolves the operator in parse analysis and
-- answers the same `42883` whatever the table holds.
-- @setup
CREATE TABLE jsonb_empty_operand (a INT);
-- @test
-- @expect-error: no integer ? integer operator over zero rows
SELECT a ? 1 FROM jsonb_empty_operand;
