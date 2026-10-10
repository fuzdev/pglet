-- The same coarser-comparator split for an indexed BIGINT: `int8 > float8` compares
-- through float8, and every key here is within one ULP of 2^53, so none is greater.
-- The accelerated `IndexRangeScan` compared them as exact i64 keys and returned
-- 9007199254740993. The bound now declines the index and the scan answers in the
-- comparator Postgres uses.
-- @profile-lock
-- @setup
CREATE TABLE bi (k BIGINT);
CREATE INDEX bi_k ON bi (k);
INSERT INTO bi VALUES (9007199254740993), (9007199254740992), (9007199254740991);
-- @param $1 float8 9007199254740992
-- @test
SELECT k FROM bi WHERE $1 > (-1)::float8 AND k > $1 ORDER BY k;
