-- A function with no argument of that name resolves no overload (`42883`).
-- @expect-error: no upper(x => unknown)
-- @test
SELECT upper(x => 'a');
