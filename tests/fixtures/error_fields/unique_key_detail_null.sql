-- `NULLS NOT DISTINCT` is the one route that puts a NULL in a `Key (…)=(…)` — it
-- renders as the bare word `null`, like a `Failing row` field.
-- @setup
CREATE TABLE t_u3 (a INT);
CREATE UNIQUE INDEX t_u3_u ON t_u3 (a) NULLS NOT DISTINCT;
INSERT INTO t_u3 VALUES (NULL);

-- @test
-- @expect-error: a second NULL collides under NULLS NOT DISTINCT
-- @no-profile-lock: same write-path enforcement — `NULLS NOT DISTINCT` changes which key tuples are candidates inside `prepared_unique_index_key`, which is ungated, not which plan the INSERT takes
-- @expect-message
INSERT INTO t_u3 VALUES (NULL);
