-- A non-UUID string literal compared against a UUID column fails loud (`22P02`),
-- matching an explicit `'notauuid'::uuid` — implicit == explicit.
-- @setup
CREATE TABLE t (id UUID);
INSERT INTO t VALUES ('11111111-1111-1111-1111-111111111111');
-- @test
-- @expect-error: unparseable string vs uuid column errors 22P02
SELECT * FROM t WHERE id = 'notauuid';
