-- `to_regclass` takes `text` only: an integer resolves no overload (`42883`).
-- @expect-error: no to_regclass(integer)
-- @test
SELECT to_regclass(1);
