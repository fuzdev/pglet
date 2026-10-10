-- No `left(bytea, integer)`: 42883.
-- @test
-- @expect-error: function left(bytea, integer) does not exist
SELECT left('\x6162'::bytea, 1);
