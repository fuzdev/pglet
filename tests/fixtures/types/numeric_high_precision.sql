-- A 60-significant-digit value round-trips exactly through the bignum.
-- @setup
CREATE TABLE t (a NUMERIC);
INSERT INTO t VALUES ('123456789012345678901234567890.123456789012345678901234567890');
-- @test
SELECT a, a * 2 AS doubled FROM t;
