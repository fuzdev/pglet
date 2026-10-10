-- The `f(*)` verdict keys on the **signature**, not on the niladic-builtin family:
-- `jsonb_build_object` carries a zero-argument entry too (`SELECT jsonb_build_object()`
-- answers `{}` on both engines), so a `*` — which contributes no argument — resolves it
-- and lands on `ParseFuncOrColumn`'s `agg_star` arm, 42809 on both. It is the only name
-- `build_call` models below its niladic block that 18.6 gives such a signature, measured
-- over the whole match; `concat(*)`, `concat_ws(*)`, `upper(*)` and `sum(*)` are 42883
-- there against pglet's 0A000, a separate loud divergence recorded in EDGES.md row 45.
-- @test
-- @expect-error: jsonb_build_object(*) specified, but jsonb_build_object is not an aggregate function
SELECT jsonb_build_object(*);
