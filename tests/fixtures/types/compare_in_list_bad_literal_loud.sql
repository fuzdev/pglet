-- An `IN` list mixing an unparseable and a parseable literal against an INT column
-- errors the WHOLE query (`22P02`) — Postgres resolves each element to the column
-- type. Previously pglet silently returned the row matching the parseable `'5'`.
-- @setup
CREATE TABLE t (n INT);
INSERT INTO t VALUES (5);
-- @test
-- @expect-error: an unparseable IN-list element errors the whole query, not a partial match
SELECT * FROM t WHERE n IN ('abc', '5');
