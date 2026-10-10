-- Pair symmetry: `GROUP BY` and `HAVING` are compared to each other by structural
-- `==` (`validate_grouped_expr` accepts a `HAVING` expression that exactly matches a
-- grouping key). Both clauses are resolved by the same pass against the same
-- `ColumnScope`, so `(x = '1')` becomes the identical `x = '1'::int4` in each and
-- still matches. Coercing one clause alone would make this query fail `42803`.
-- @setup
CREATE TABLE gt (x INT, tag TEXT);
INSERT INTO gt VALUES (1, 'a'), (1, 'b'), (2, 'c');
-- @test
SELECT count(*) AS n FROM gt GROUP BY (x = '1') HAVING (x = '1');
