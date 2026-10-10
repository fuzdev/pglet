-- A `nextval` body beside a data-modifying CTE that takes the same sequence: 18.6 runs
-- the body first (it answers `1` and the CTE's row takes `2`); run after the CTE it would
-- answer `2`. Refused.
-- @setup
CREATE TABLE t (id SERIAL, v TEXT);
-- @expect-error: pglet refuses a sequence function beside a data-modifying CTE (18.6 answers)
-- @test
WITH w AS (INSERT INTO t (v) VALUES ('w') RETURNING id) SELECT nextval('t_id_seq');
