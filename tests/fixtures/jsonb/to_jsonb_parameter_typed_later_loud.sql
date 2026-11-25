-- A `$N` typed only by an occurrence 18.6 analyses after `to_jsonb`'s — a `WHERE` after
-- the select list — leaves it nothing to resolve there (`42804`).
-- @param $1 int4 1
-- @expect-error: could not determine polymorphic type
-- @test
SELECT to_jsonb($1) WHERE $1 = 1;
