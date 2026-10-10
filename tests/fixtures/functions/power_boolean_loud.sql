-- No `power(boolean, integer)`: 42883.
-- @test
-- @expect-error: function power(boolean, integer) does not exist
SELECT power(true, 2);
