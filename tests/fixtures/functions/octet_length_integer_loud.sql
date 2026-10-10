-- No `octet_length(integer)`: 42883.
-- @test
-- @expect-error: function octet_length(integer) does not exist
SELECT octet_length(123);
