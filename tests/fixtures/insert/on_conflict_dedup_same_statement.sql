-- Two rows in the same VALUES list share the arbiter key: the second must see
-- the first as a pending row (read-your-own-writes within one statement) and DO
-- NOTHING, leaving exactly one row with the first row's values. This exercises
-- the auto-commit write path's pending-index probe (the implicit-transaction
-- wrap): the intra-statement conflict is resolved against staged, not-yet-
-- committed rows, matching Postgres.
-- @profile-lock
-- @setup
CREATE TABLE t (id INT PRIMARY KEY, name TEXT);
INSERT INTO t VALUES (10, 'first'), (10, 'second') ON CONFLICT (id) DO NOTHING;

-- @test
SELECT * FROM t ORDER BY id;
