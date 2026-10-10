-- A non-numeric string literal compared against a NUMERIC column fails loud
-- (`22P02`), matching an explicit `'x'::numeric`.
-- @setup
CREATE TABLE t (n NUMERIC);
INSERT INTO t VALUES (1.5);
-- @test
-- @expect-error: unparseable string vs numeric column errors 22P02
SELECT * FROM t WHERE n = 'x';
