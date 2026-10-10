-- `bytea = text` has no operator (`42883`) — Postgres registers no implicit path
-- between the two, so the pair must fail rather than compare by rendering.
-- @setup
CREATE TABLE t (b BYTEA, s TEXT);
INSERT INTO t VALUES ('\x6162', 'ab');
-- @test
-- @expect-error: no bytea = text operator
SELECT * FROM t WHERE b = s;
