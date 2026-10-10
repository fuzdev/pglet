-- Outside a transaction block `CONCURRENTLY` builds an ordinary index and drops
-- one, in `gram.y`'s keyword order (`INDEX CONCURRENTLY IF NOT EXISTS` for the
-- create, `INDEX CONCURRENTLY IF EXISTS` for the drop). `@no-txn` because the
-- harness's wrapper would otherwise make every statement here the `25001` its
-- sibling fixtures assert.
-- @no-txn
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS tic;
CREATE TABLE tic (id INT, v TEXT);
INSERT INTO tic VALUES (1, 'a'), (2, 'b');
CREATE INDEX CONCURRENTLY tic_id_idx ON tic (id);
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS tic_v_idx ON tic (v);
DROP INDEX CONCURRENTLY tic_id_idx;
DROP INDEX CONCURRENTLY IF EXISTS tic_nope_idx;
-- @test
SELECT id, v FROM tic WHERE v = 'b';
