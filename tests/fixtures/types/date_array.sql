-- A `date[]` column: the array element path carries the Date element type and
-- renders as the Postgres `{...}` literal — differential vs Postgres.
-- @setup
CREATE TABLE t (id INT, ds DATE[]);
INSERT INTO t VALUES (1, '{2024-01-01,2024-02-02}'), (2, '{}');

-- @test
SELECT id, ds FROM t ORDER BY id;
