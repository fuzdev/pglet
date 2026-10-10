-- length() has no integer overload — a non-text argument resolves to no function
-- (42883, undefined_function) in both pglet and Postgres, rather than silently
-- coercing the integer to text.
-- @test
-- @expect-error: function length(integer) does not exist
SELECT length(5);
