-- The bound form of the projection-unnest: `$1` broadcast, `unnest($2::text[])`
-- row-expanded. Exercises parameter binding through `Expr::Unnest` — `$2`'s type
-- is inferred from its `::text[]` cast and the decoded array drives the expansion.
-- Exactly the spine's `query_put_fact_refs` projection (`SELECT $1, unnest($2)`).
-- @param $1 text src
-- @param $2 text[] {t1,t2,t3}
-- @test
SELECT $1::text AS s, unnest($2::text[]) AS u ORDER BY u;
