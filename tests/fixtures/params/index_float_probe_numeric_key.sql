-- A float8-bound parameter against an indexed NUMERIC column. Postgres compares
-- `numeric = float8` by casting the numeric to float8, so all three rows collapse to
-- `0.3` and match. pglet's index key for NUMERIC is **exact**, so an accelerated
-- probe on the coerced key returned only the row that is literally `0.3` — a
-- different answer from the stripped profile's `Filter(Scan)`. The probe now declines
-- the pairs whose scan comparator is coarser than the key order.
-- @profile-lock
-- @setup
CREATE TABLE pf (n NUMERIC);
CREATE INDEX pf_n ON pf (n);
INSERT INTO pf VALUES ('0.30000000000000000001'), ('0.3'), ('0.29999999999999999999');
-- @param $1 float8 0.3
-- @test
SELECT n FROM pf WHERE $1 > (-1)::float8 AND n = $1 ORDER BY n;
