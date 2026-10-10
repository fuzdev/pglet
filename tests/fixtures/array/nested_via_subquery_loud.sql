-- The nesting guard cannot live in the parser alone: an element that is only *known* to
-- be an array once evaluated reaches the same state through a subquery. Postgres types
-- this `integer[]`; pglet used to declare `text` and build a 1-element text array.
-- @test
-- @expect-error: nested arrays unsupported, reached through a subquery element
SELECT ARRAY[(SELECT ARRAY[1,2])]::TEXT;
