-- A scalar subquery's value is read through a node whose datum pglet does not place, so
-- it is refused `0A000`. Documentary where 18.6 answers (any pglet error passes).
-- @setup
CREATE TABLE pcq (b bytea);
INSERT INTO pcq VALUES ('\xdeadbeef');
-- @test
-- @expect-error: pg_column_size of a subquery
SELECT pg_column_size((SELECT b FROM pcq));
