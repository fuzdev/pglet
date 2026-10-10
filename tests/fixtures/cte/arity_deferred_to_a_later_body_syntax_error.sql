-- The ordinary route's arity check is **deferred** to the end of the `WITH` list, so a
-- later body's syntax error still wins — which is where 18.6 raises it, its raw parse
-- running before every `analyzeCTE`. Raised at the declaration, this was `42P10`.
-- @expect-error: a later body syntax error outranks the deferred arity check
-- @no-error-fields: both engines send a `P` and disagree about which token is wrong —
-- 18.6 accepts an empty target list (`SELECT FROM x` is grammatical there) and stops at
-- the second `FROM`, pglet's expression parser at the first. The SQLSTATE agrees.
-- @test
WITH c(a, b) AS (SELECT 1), q AS (SELECT FROM FROM) SELECT 1 AS only_column;
