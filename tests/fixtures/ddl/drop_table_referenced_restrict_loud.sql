-- RESTRICT is SQL's default drop behavior, so it must behave exactly like the bare
-- form: still 2BP01, not a silent pass through a parsed-and-ignored keyword.
-- @setup
CREATE TABLE dtrr_par (id INT PRIMARY KEY);
CREATE TABLE dtrr_ch (cid INT PRIMARY KEY, pid INT REFERENCES dtrr_par(id));

-- @expect-error: cannot drop table dtrr_par because constraint dtrr_ch_pid_fkey depends on it
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP TABLE dtrr_par RESTRICT;
