-- A `(…)` type modifier on a type that carries none is **refused**, not dropped.
-- Membership is the type's own `pg_type.typmodin` — `int4`'s is 0 — so parse
-- analysis raises `42601` naming the type as spelled (`typenameTypeMod`,
-- `parse_type.c`) instead of answering the unmodified `1`.
-- @expect-error: int4 takes no type modifier
-- @expect-message
-- @test
SELECT 1::int4(3) AS never_reached;
