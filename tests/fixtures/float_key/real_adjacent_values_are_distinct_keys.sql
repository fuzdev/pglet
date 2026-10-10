-- Two `real` values one step apart are two keys: a unique key takes both, and a probe
-- for either finds only its own row. A key that kept less than the value's full
-- precision would fold them.
-- @profile-lock
-- @setup
CREATE TABLE fkadj (k INT, x REAL UNIQUE);
INSERT INTO fkadj VALUES (1, 1.1), (2, 1.1000001), (3, 16777216), (4, 16777218);
-- @param $1 float8 1.1000001430511475
-- @test
SELECT k FROM fkadj WHERE x = $1::float8 ORDER BY k;
