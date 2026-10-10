-- The list renames and never retypes — `analyzeCTE` reads each column's type from
-- `exprType(te->expr)`. Both declared-type legs compare the OIDs live.
-- @test
WITH c(v, w) AS (SELECT 'abc'::text, 5::int8) SELECT v, w FROM c;
