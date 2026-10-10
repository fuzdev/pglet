-- A `WITH RECURSIVE` item with no combinator and no self-reference is an ordinary CTE,
-- so its arity check is deferred like the ordinary route's and a later body's syntax
-- error wins. Its `UNION` sibling is
-- `with_recursive_list/arity_deferred_on_the_union_degrade.sql`.
-- @expect-error: a later body syntax error outranks the deferred arity check
-- @no-error-fields: the same `42601` position split as the ordinary route's twin.
-- @test
WITH RECURSIVE r(n, m) AS (SELECT 1), q AS (SELECT FROM FROM) SELECT 1 AS only_column;
