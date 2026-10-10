-- @setup
CREATE TABLE ssqd1 (c INT);
CREATE TABLE ssqd2 (k INT);
INSERT INTO ssqd2 VALUES (1), (2);

-- The refuse polarity of `nested_sort_key_equal_scalar_subqueries_resolve.sql`. Two
-- scalar-subquery entries under one output name whose sub-plans differ are **not**
-- `equal()`, so they are two claimants and the bare key is `42702` — the check that the
-- accept half is a real comparison and not a plan-bearing entry being waved through.
-- @expect-error: two different scalar subqueries under one output name are 42702
-- @test
SELECT k FROM ssqd2 WHERE EXISTS (SELECT (SELECT 1) AS q, (SELECT 2) AS q FROM ssqd1 WHERE false ORDER BY q);
