-- A negative-scale NUMERIC(p,-s) column rounds to a power of ten on store.
-- @setup
CREATE TABLE t (id INT, a NUMERIC(5,-2));
INSERT INTO t VALUES (1, 12345.6), (2, 999), (3, 40);
-- @test
SELECT id, a FROM t ORDER BY id;
