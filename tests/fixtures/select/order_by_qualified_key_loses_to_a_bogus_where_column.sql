-- Error order, second of three: `transformWhereClause` runs before `transformSortClause`,
-- so the `WHERE`'s missing column outranks the sort key's missing relation (18.6: `42703
-- column "bogusw" does not exist`; this was the key's `42P01`). See
-- `select/order_by_qualified_key_loses_to_a_bogus_projection_column.sql` for the class.
-- @setup
CREATE TABLE sk_epw (a INT, s TEXT);
INSERT INTO sk_epw VALUES (1, 'x'), (2, 'y');

-- @expect-error: column "bogusw" does not exist (42703)
-- @test
SELECT a FROM sk_epw WHERE bogusw = 1 ORDER BY bogusrel.a;
