-- One series column and its ordinality: a third alias is `42P10`.
-- @expect-error: more aliases than the series and its ordinality provide
-- @expect-message
-- @test
SELECT * FROM generate_series(1, 2) WITH ORDINALITY AS g(v, n, z);
