-- A nested path literal, which Postgres flattens into the path a, b, fails loud here
-- rather than naming the keys `{a` and `b}` and answering NULL.
-- @test
-- @expect-error: pglet does not support multidimensional arrays (Postgres accepts this)
SELECT '{"a":{"b":1}}'::jsonb #> '{{a,b}}';
