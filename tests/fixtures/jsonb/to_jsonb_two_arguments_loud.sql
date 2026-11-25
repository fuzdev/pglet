-- `to_jsonb` takes one argument (`42883`).
-- @expect-error: no to_jsonb(integer, integer)
-- @test
SELECT to_jsonb(1, 2);
