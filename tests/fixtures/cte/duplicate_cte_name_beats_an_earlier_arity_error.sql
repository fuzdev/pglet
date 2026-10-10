-- `transformWithClause`'s duplicate scan runs before every `analyzeCTE`, so a `42712`
-- outranks a `42P10` an **earlier** declaration would raise. Both verdicts are recorded
-- and fired after the whole list has parsed, in that order; raised where they are found,
-- declaration 1's arity error came first and won.
-- @expect-error: the duplicate name outranks the earlier declaration's arity error
-- @expect-message
-- @test
WITH c(a, b) AS (SELECT 1), c AS (SELECT 2) SELECT 1 AS only_column;
