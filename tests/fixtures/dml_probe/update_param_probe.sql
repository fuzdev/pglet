-- A bound key probes the primary key, and a bound residual parameter is still tested
-- against each candidate.
-- @profile-lock
-- @param $1 int4 2
-- @param $2 text b
-- @setup
CREATE TABLE pp (id INT PRIMARY KEY, v TEXT);
INSERT INTO pp VALUES (1, 'a'), (2, 'b'), (3, 'b');
-- @test
UPDATE pp SET v = 'x' WHERE id = $1 AND v = $2 RETURNING id, v;
