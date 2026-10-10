-- Comparison operators are non-associative in Postgres (`%nonassoc`): a chained
-- `1 = 1 = true` is a syntax error, not `(1 = 1) = true`. SQLSTATE 42601. (The
-- same holds for the mixed form `1 < 2 = true` and the other four operators —
-- covered by the parser unit tests.)
-- @test
-- @expect-error: comparison operators are not associative
SELECT 1 = 1 = true;
