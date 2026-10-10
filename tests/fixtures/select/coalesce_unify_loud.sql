-- A string literal that can't parse as the unified branch type fails loud at
-- runtime, matching Postgres (`invalid input syntax for type integer: "foo"`).
-- Before branch unification, pglet silently returned the raw text instead.
-- @expect-error: COALESCE(int, 'foo') coerces 'foo'→int and fails to parse
-- @setup
CREATE TABLE unif_loud (id INT, n INT);
INSERT INTO unif_loud VALUES (1, NULL);
-- @test
SELECT COALESCE(n, 'foo') FROM unif_loud;
