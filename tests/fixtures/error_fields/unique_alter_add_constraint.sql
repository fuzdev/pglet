-- `ALTER TABLE … ADD UNIQUE` over duplicate rows is an **index build**, not a row
-- write: 18.6 says `could not create unique index` and its DETAIL says *duplicated*,
-- not *already exists*.
-- @setup
CREATE TABLE t_uadd (x INT);
INSERT INTO t_uadd VALUES (1), (1);

-- @test
-- @expect-error: existing rows are not distinct
-- @profile-lock
-- @expect-message
ALTER TABLE t_uadd ADD CONSTRAINT t_uadd_u UNIQUE (x);
