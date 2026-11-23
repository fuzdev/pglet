-- A dropped `CHECK` — column-level under its generated name, table-level under a
-- declared one — no longer refuses a row that violates it.
-- @setup
CREATE TABLE dcck (a INT CHECK (a > 0), b INT, CONSTRAINT dcck_b_small CHECK (b < 10));
INSERT INTO dcck VALUES (1, 1);

-- @test
ALTER TABLE dcck DROP CONSTRAINT dcck_a_check, DROP CONSTRAINT dcck_b_small;
INSERT INTO dcck VALUES (-5, 50);
SELECT a, b, (SELECT count(*) FROM pg_constraint WHERE conname LIKE 'dcck%') AS checks
FROM dcck ORDER BY a;
