-- A bare `$N` nothing types leaves `to_jsonb(anyelement)` no type to resolve (`42804`).
-- @param $1 int4 1
-- @expect-error: could not determine polymorphic type
-- @test
SELECT to_jsonb($1);
