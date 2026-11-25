-- A bare string literal in a field reads as `integer`, and `'1.5'` does not.
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT make_interval(hours => '1.5');
