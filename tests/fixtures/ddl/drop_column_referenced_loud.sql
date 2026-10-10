-- Dropping a **referenced** (parent-side) column is refused 2BP01: the FK's edge to
-- it is NORMAL, so RESTRICT will not follow it. pglet reported the index
-- position-drift guard's 0A000 here instead, which named the wrong cause — the
-- dependency check now runs first so the FK wins, as in Postgres.
-- @setup
CREATE TABLE dcr_par (id INT PRIMARY KEY, code TEXT);
CREATE TABLE dcr_ch (cid INT PRIMARY KEY, pid INT REFERENCES dcr_par(id));

-- @expect-error: cannot drop column id of table dcr_par because constraint dcr_ch_pid_fkey depends on it
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE dcr_par DROP COLUMN id;
