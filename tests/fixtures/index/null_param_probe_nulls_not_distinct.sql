-- The bound spelling of `null_key_probe_nulls_not_distinct`: `u = $1` with `$1` NULL
-- matches no row over a `NULLS NOT DISTINCT` unique index.
-- @profile-lock
-- @param $1 int4 NULL
-- @setup
CREATE TABLE nq (id INT, u INT);
CREATE UNIQUE INDEX nq_u ON nq (u) NULLS NOT DISTINCT;
INSERT INTO nq VALUES (1, NULL), (2, 5);
-- @test
SELECT id FROM nq WHERE u = $1;
