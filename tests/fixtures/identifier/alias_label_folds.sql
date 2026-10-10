-- An `AS` label is the output column's **name**, so it folds like any other unquoted
-- identifier and only a quoted label keeps its case: `bar`, `Baz`, `bazz`. This is a
-- silent divergence when it is wrong — a client keying rows by column name
-- (node-postgres does) reads `row.Bar` from an unfolded engine and `row.bar` from
-- Postgres, with no error anywhere — and it is why the fixture carries both
-- polarities in one statement: an over-correction that folded the quoted label too
-- would satisfy the first and third columns and redden the second.
-- @test
SELECT 1 AS Bar, 2 AS "Baz", 3 AS BAZZ;
