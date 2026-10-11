-- A computed argument is one pglet does not place: 18.6 sizes the datum its expression
-- built (`t || ''` is 9 for a stored `'hello'`, which is 6 read off the row), and pglet
-- refuses `0A000` rather than reason about which expressions pass a stored datum through.
-- Documentary where 18.6 answers (any pglet error passes); it holds the refusal against
-- an answer.
-- @setup
CREATE TABLE pcc (t text);
INSERT INTO pcc VALUES ('hello');
-- @test
-- @expect-error: pg_column_size of a computed argument
SELECT pg_column_size(t || '') FROM pcc;
