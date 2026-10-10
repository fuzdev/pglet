-- An `IN` list whose bad element comes AFTER a matching one (`n = 42` matches) still
-- errors the whole query (`22P02`) — Postgres resolves/validates every constant list
-- element at parse time, so the error is independent of match order. Previously pglet
-- short-circuited on the match and silently returned the row.
-- @setup
CREATE TABLE t (n INT);
INSERT INTO t VALUES (42);
-- @test
-- @expect-error: an unparseable IN-list element after a match still errors the query
SELECT * FROM t WHERE n IN (42, 'abc');
