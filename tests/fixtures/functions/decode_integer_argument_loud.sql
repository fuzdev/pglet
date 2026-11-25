-- `decode` takes text: an integer resolves no overload (`42883`).
-- @expect-error: no decode(integer, unknown)
-- @test
SELECT decode(1, 'hex');
