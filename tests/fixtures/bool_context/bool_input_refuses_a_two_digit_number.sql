-- `1` and `0` are accepted at length **exactly** one, which is what refuses `01`.
-- @expect-error: invalid input syntax for type boolean
-- @test
SELECT '01'::bool;
