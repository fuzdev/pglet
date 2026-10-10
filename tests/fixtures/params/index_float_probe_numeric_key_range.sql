-- The range half of `params/index_float_probe_numeric_key.sql`: `numeric > float8`
-- compares through float8, where every row is exactly `0.3`, so nothing is greater.
-- An accelerated `IndexRangeScan` on the exact numeric key found the row that is
-- greater *as a numeric* and returned it. The bound now declines the index.
-- @profile-lock
-- @setup
CREATE TABLE pfr (n NUMERIC);
CREATE INDEX pfr_n ON pfr (n);
INSERT INTO pfr VALUES ('0.30000000000000000001'), ('0.3'), ('0.29999999999999999999');
-- @param $1 float8 0.3
-- @test
SELECT n FROM pfr WHERE $1 > (-1)::float8 AND n > $1 ORDER BY n;
