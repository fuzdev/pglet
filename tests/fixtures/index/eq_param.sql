-- Index point read with a BOUND parameter: `WHERE k = $1` over the extended
-- protocol (the spine's real binding path). The planner lifts `k = $1` into an
-- IndexScan whose `key_value` is the `$N`; param type is inferred from the
-- indexed column, and the bound probe must return the same rows as Postgres.
-- @profile-lock
-- @setup
CREATE TABLE idx_param (id INT, k INT, name TEXT);
INSERT INTO idx_param VALUES
  (1, 100, 'x'),
  (2, 200, 'y'),
  (3, 100, 'z');
CREATE INDEX idx_param_k ON idx_param(k);
-- @param $1 int4 100
-- @test
SELECT id, k, name FROM idx_param WHERE k = $1 ORDER BY id;
