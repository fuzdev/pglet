-- No `md5(integer)`: 42883.
-- @test
-- @expect-error: function md5(integer) does not exist
SELECT md5(1);
