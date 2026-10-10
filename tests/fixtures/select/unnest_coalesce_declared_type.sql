-- A FROM-clause `unnest(…)` is an ordinary named relation, and its columns' **types**
-- have to reach the plan-time coercion pass — not just their names. `COALESCE` unifies
-- its branches there (`transformCoalesceExpr` → `select_common_type`), and that
-- unification is the only thing that types an untyped `NULL` branch. pglet registered
-- the unnest under its alias with an *empty* column map, so no branch unified and the
-- executor's own type walk — which is total, and takes a `COALESCE`'s first argument's
-- type — fell back to `text`: two int4 values went out under OID 25 on the simple-query
-- `RowDescription` while `Describe` said int4 (23) for the same statement, which is what
-- 18.6 says on both legs.
--
-- `leading_typed` is the polarity pin. `COALESCE(v, 0)` was already int4 on both legs
-- (its first branch is the typed one), so a fix that changed the `COALESCE` result rule
-- rather than the scope its branches resolve against reddens here.
--
-- The same registration gap is why a *qualified* reference to an unnest column and
-- `alias.*` over one still diverge; those are separate recorded classes
-- (`EDGES.md`) and this fixture pins neither.
-- @test
SELECT COALESCE(NULL, v) AS leading_null, COALESCE(v, 0) AS leading_typed
FROM unnest(ARRAY[1, 2]) AS u(v);
