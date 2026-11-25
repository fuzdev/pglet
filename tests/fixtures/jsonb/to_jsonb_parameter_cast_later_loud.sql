-- ...and so does a cast later in the same select list.
-- @param $1 int4 1
-- @expect-error: could not determine polymorphic type
-- @test
SELECT to_jsonb($1), $1::int;
