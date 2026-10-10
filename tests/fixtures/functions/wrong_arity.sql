-- upper() takes exactly one argument — a wrong argument count resolves to no
-- function (42883, undefined_function) in both pglet and Postgres.
-- @test
-- @expect-error: function upper(text, text) does not exist
SELECT upper('a', 'b');
