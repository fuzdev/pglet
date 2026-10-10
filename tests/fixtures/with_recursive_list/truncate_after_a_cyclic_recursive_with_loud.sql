-- A `TRUNCATE` after a list 18.6 would refuse as mutual recursion: the raw parse's
-- syntax error comes first.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error at TRUNCATE (42601)
-- @test
WITH RECURSIVE a AS (SELECT * FROM b), b AS (SELECT * FROM a) TRUNCATE s;
