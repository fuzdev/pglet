-- A bare DROP TABLE on a parent an external FK references is refused (2BP01) —
-- pglet used to drop it and leave the child's FK entry pointing at a vanished
-- table, so every later write to the child failed 23503 naming it.
-- @setup
CREATE TABLE dtr_par (id INT PRIMARY KEY);
CREATE TABLE dtr_ch (cid INT PRIMARY KEY, pid INT REFERENCES dtr_par(id));

-- @expect-error: cannot drop table dtr_par because constraint dtr_ch_pid_fkey depends on it
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dtr_par;
