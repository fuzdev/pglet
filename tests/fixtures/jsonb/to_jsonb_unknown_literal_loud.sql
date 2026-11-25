-- A bare string literal gives `to_jsonb(anyelement)` no type to resolve (`42804`).
-- @expect-error: could not determine polymorphic type
-- @test
SELECT to_jsonb('x');
