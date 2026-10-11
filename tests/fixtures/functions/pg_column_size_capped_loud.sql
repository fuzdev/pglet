-- A length-capped character column may be a `char(n)`, which 18.6 stores blank-padded to
-- `n` and pglet unpadded (`EDGES.md` row 95), so pglet refuses `0A000` for either
-- spelling rather than size the unpadded value. Documentary where 18.6 answers.
-- @setup
CREATE TABLE pcv (c char(10), v varchar(10));
INSERT INTO pcv VALUES ('ab', 'ab');
-- @test
-- @expect-error: pg_column_size of a length-capped column
SELECT pg_column_size(c) FROM pcv;
