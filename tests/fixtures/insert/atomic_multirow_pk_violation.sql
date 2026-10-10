-- A multi-row INSERT whose middle row duplicates an existing primary key aborts
-- the whole statement (Postgres per-statement atomicity), with SQLSTATE 23505.
-- This fixture pins the SQLSTATE; the surviving-rows atomicity (no partial rows
-- left applied) is covered by the `autocommit_atomicity` Rust integration test —
-- a single `@test` can't assert post-error state.
-- @profile-lock
-- @setup
CREATE TABLE t (id INT PRIMARY KEY, u INT UNIQUE, v TEXT);
INSERT INTO t VALUES (1, 10, 'a'), (2, 20, 'b');

-- @test
-- @expect-error: mid-batch duplicate primary key aborts the whole statement
INSERT INTO t VALUES (3, 30, 'c'), (1, 40, 'd'), (4, 50, 'e');
