-- A string literal the resolved type cannot read is that type's 22P02.
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT * FROM generate_series(1, 'x');
