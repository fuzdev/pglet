-- @setup
CREATE TABLE t1 (num INT, name TEXT);
CREATE TABLE t2 (num INT, value TEXT);
INSERT INTO t1 VALUES (1, 'one'), (2, 'two');
INSERT INTO t2 VALUES (1, 'a'), (2, 'b');

-- @test
-- @expect-error: NATURAL JOIN is unsupported (pglet-stricter; Postgres accepts it)
SELECT * FROM t1 NATURAL JOIN t2;
