-- A non-integer `->` key resolves to no operator. pglet used to return NULL.
-- @test
-- @expect-error: no jsonb -> numeric
SELECT '[1,2,3]'::jsonb -> 1.0;
