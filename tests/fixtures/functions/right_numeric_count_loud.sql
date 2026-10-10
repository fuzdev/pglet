-- No `right(text, numeric)`: 42883.
-- @test
-- @expect-error: function right(unknown, numeric) does not exist
SELECT right('abc', 1.5);
