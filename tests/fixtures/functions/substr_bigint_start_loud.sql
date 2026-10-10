-- No `substr(text, bigint)`: 42883.
-- @test
-- @expect-error: function substr(unknown, bigint) does not exist
SELECT substr('abc', 1::int8);
