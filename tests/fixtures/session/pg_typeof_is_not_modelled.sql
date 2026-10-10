-- 18.6 answers `integer` here; pglet does not model `pg_typeof` at all, because
-- declaring it correctly needs `regtype` (whose binary form is a 4-byte OID and
-- whose text form is a type name) and because its *value* is a static type OID —
-- the question pglet's two type walkers answer separately (`docs/declared_types.md`).
-- The refusal is loud, which is what this pins: the alternative to modelling it is
-- `0A000`, never a fabricated type name.
-- @expect-error: pglet-stricter — pg_typeof is unmodelled and fails loud
-- @test
SELECT pg_typeof(1);
