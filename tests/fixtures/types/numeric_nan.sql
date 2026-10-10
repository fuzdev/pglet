-- NUMERIC 'NaN' round-trips, sorts above every finite value, and equals itself
-- (Postgres numeric semantics, unlike float NaN).
-- @setup
CREATE TABLE t (id INT, a NUMERIC);
INSERT INTO t VALUES (1, 'NaN'), (2, 100), (3, -5), (4, 'NaN');

-- @test
SELECT id, a FROM t ORDER BY a, id;
