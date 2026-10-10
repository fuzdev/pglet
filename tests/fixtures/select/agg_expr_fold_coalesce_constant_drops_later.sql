-- `COALESCE` arguments after an integer, `numeric` or boolean literal are removed with
-- their aggregates — the literal bare, or under the cast that brings it to the
-- `COALESCE`'s type — also behind an argument that is no constant. A `NULL` removes
-- nothing.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT, n NUMERIC, f FLOAT8, bo BOOL);
INSERT INTO u VALUES (1, 10, 'x', 1.5, 2.5, true), (2, 0, 'z', 2.5, 3.5, false);
-- @test
SELECT coalesce(1, sum(1/b)) AS one, coalesce(NULL::int, 2, sum(1/b)) AS two,
  coalesce(sum(a), 1, sum(1/b)) AS three,
  coalesce(1.5, sum(1/b)) AS numeric_literal,
  coalesce(1, sum(n / b)) AS to_numeric,
  coalesce(1, sum(f / b)) AS to_float,
  coalesce(true, max(bo::int / b) > 0) AS bool_literal,
  coalesce(10000000000, sum(1/b)) AS bigint_literal
FROM u;
