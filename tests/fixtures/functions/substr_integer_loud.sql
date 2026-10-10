-- No `substr(integer, integer)`: 42883.
-- @test
-- @expect-error: function substr(integer, integer) does not exist
SELECT substr(123, 1);
